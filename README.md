# Keepframe 0.2 beta

Keepframe is a local-first Windows photo catalogue for safely organising, triaging and mapping a personal archive, then preparing and tracking AI-assisted restorations. It is deliberately not a conventional RAW developer: protected originals, catalogue decisions and derived edits remain separate.

> Keepframe is a provisional beta name. Do not publish this build commercially until naming clearance and the release gates in `RELEASE_GATES.md` are complete.

## Principal workflow

1. Choose or reconnect a portable master-library folder.
2. Import with **Copy** (default) or explicitly choose **Move after verification**.
3. Browse the timeline, tags and clustered full-library map; search configured places, select a marker, or place an unlocated photograph without changing its source metadata.
4. Triage with `M` to Keep, `X` to Discard and the arrow keys to browse.
5. Move discarded photographs to reversible Keepframe Trash. Empty Trash is separately confirmed.
6. Use Develop for non-destructive exposure, white balance, tonal, presence, colour, transform, crop and local-mask edits; optional local intelligent masking can propose Subject, combined People or Sky coverage for explicit acceptance into the same authoritative recipe.
7. Choose a plain-language AI action such as **Improve photo**, **Improve lighting**, **Enhance colour**, **Restore old photo**, **Remove distraction** or **Custom instruction**.
8. Choose a local edit when it is genuinely available, or prepare a manual ChatGPT/Gemini hand-off. Import the returned image as a traceable candidate version.

Catalogue, triage and manual external-edit workflows work without either optional AI service. Keepframe does not submit to ChatGPT or Gemini APIs and stores no cloud API keys.

## Safety model

- Copy and duplicate retention are always the import defaults.
- A staged file and final managed file must match the source SHA-256 before catalogue registration.
- Move removes a source only after managed placement, verification and catalogue commit, followed by an immediate recheck of both source and managed copy.
- Unsupported, changed, missing or failed source files are retained.
- Discard is only a catalogue decision. Trash is internal and reversible; Empty Trash uses the Windows Recycle Bin.
- SQLite runs in WAL mode. Schema upgrades create and integrity-check a SQLite-consistent backup. Backups are written as partial files, verified, then promoted atomically.
- Each library has a UUID marker and an exclusive lock. Missing/corrupt libraries open recovery rather than silent first-run setup.
- Browser media access is restricted to generated previews and derived edits inside the current library. RAW files are not served to the WebView.

If Keepframe restarts after an interrupted import, its import record is marked **needs attention** and all staged/managed files are retained. After interrupted Trash or Restore work, Keepframe reconciles only its catalogue state and tells you to inspect Trash or the Windows Recycle Bin. It does not delete files to make recovery appear complete.

## Clean-checkout validation

Requirements: Windows 10/11, WebView2, Node.js 24+, pnpm 11+, Rust 1.96+ and Python 3.11+.

```powershell
pnpm install
pnpm check:all
```

The command runs React tests, TypeScript/Vite production build, Rust tests, strict Clippy and lightweight Python schema/segmentation-helper tests. It creates `ai-worker\.test-venv` with Pydantic and Pillow only; PyTorch and model weights are not installed.

Run a disposable native profile:

```powershell
$env:KEEPFRAME_SETTINGS_DIR = "$PWD\.test-profile"
pnpm tauri dev
```

Build the invitation-only NSIS beta and generate its matching checksum/provenance:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-installer.ps1
```

The current local installer is copied to `release\`; use only the filename and SHA-256 recorded in `release\BUILD_PROVENANCE.json` and `release\SHA256SUMS.txt`.

## Test fixtures

Rust tests generate disposable JPEG, PNG and TIFF image fixtures and verify metadata dimensions, thumbnail generation and corrupted-image rejection. Supported RAW extensions include CR2/CR3, NEF and ARW, but their decoder path depends on the bundled LibRaw tool and camera-specific files. We do not commit third-party camera originals without a clear redistribution licence. For pre-release camera coverage, place legally approved CR2/CR3, NEF and ARW samples in a private directory and run:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\test-raw-fixtures.ps1 -FixtureRoot D:\Keepframe-private-fixtures
```

The fixture smoke test verifies EXIF dimensions/model and LibRaw inspection without modifying the originals. Then perform Copy import, thumbnail/review generation and malformed-file rejection against each fixture; record the camera model and LibRaw result in beta evidence. This is a residual beta gate, not a claim of universal RAW compatibility.

## Location and map browsing

Map markers use an independent lightweight catalogue query, so they represent all geotagged photographs matching the active catalogue filters rather than only the current library page. Nearby markers cluster at wider zoom levels. **Use visible area** creates an explicit temporary spatial filter; clear it to return to the wider catalogue.

Keepframe preserves imported embedded GPS separately from a manual catalogue pin. Moving or clearing a manual pin never writes source EXIF and restores the embedded point where available. Tile providers and optional place search are build-time configurable; no geocoder is enabled by default. See `MILESTONE_4_LOCATION_GUIDE.md` for configuration, privacy and offline behaviour.

## Portability and library resilience

**Settings & Library Health** provides explicit local XMP sidecar export/import, a versioned portable catalogue JSON export, integrity/rescan, hash-confirmed relinking and an optional Changes-detected Inbox for folders you choose to watch. These operations do not rewrite original image pixels, auto-import files, delete catalogue records or silently relink an uncertain match. XMP and catalogue exports may include GPS coordinates, paths, filenames, tags and hashes, so share them deliberately.

See `MILESTONE_5_INTEROPERABILITY_GUIDE.md` for the metadata mapping, XMP subset, export schema, moved-library path rebasing, folder-watch behaviour and known limits.

See `MILESTONE_6_DEVELOP_GUIDE.md` for Develop controls, render order, recipe persistence, cache behaviour, shortcuts and current RAW/colour boundaries.

See `MILESTONE_9_INTELLIGENT_MASKING_GUIDE.md` for the optional local segmentation model, explicit installation, accepted-mask format, privacy, provider behaviour, qualification and limitations.

See `MILESTONE_10_RENDERER_PERFORMANCE_GUIDE.md` for reproducible cold/warm renderer benchmarks, profiling evidence, cache ownership and invalidation, CPU/GPU decisions, and the native acceptance checklist.

## Optional local AI

The existing image-edit service defaults to `http://127.0.0.1:7868`. Keepframe rejects non-loopback service addresses. Optional Qwen3-VL analysis is installed separately; no model downloads automatically.

```powershell
.\scripts\setup-ai-worker.ps1
.\scripts\download-analysis-model.ps1
.\scripts\download-segmentation-model.ps1
```

The runtime may require several GB; it is installed at `D:\AI Models\Keepframe\runtime`. The optional Qwen analysis model is approximately 17.5 GB. The separate BEiT intelligent-masking model is approximately 900 MB. Both downloads require an explicit action. Every weight, cache and companion-worker asset is constrained beneath `D:\AI Models\Keepframe`.

When the vision model is absent, Keepframe labels the recipe as a deterministic controls-only fallback. Qwen-Image-Edit at port 7868 is a separate integration and is reported independently in Settings & Health.

## AI workflow and availability

The AI Workshop records a small structured recipe behind each plain-language action: the goal, constraints to preserve, restrictions, edit strength and an optional custom instruction. Provider prompts are rendered deterministically from that recipe; the default workshop view does not require editing prompt text or JSON.

Settings shows the loopback-only service address and a bounded health result: available, service not running, model missing, incompatible configuration or health-check failure. Health results are briefly cached; use **Refresh status** after changing a local service or model. Keepframe never marks local editing ready without the required Qwen image-edit capability.

For ChatGPT and Gemini, **Prepare image and instruction** creates a local sRGB PNG, a provider-specific instruction and a persistent **waiting for external result** job. Keepframe never uploads the photograph. After you explicitly upload and edit it with the chosen provider, **Import returned image** validates that the file exists, decodes, has plausible dimensions and is not identical to the protected source. The result is stored under `Edits` as an AI-derived candidate with provider, prompt, recipe, hashes and time. You can compare it with, prefer it over, or return to the protected original at any time.

## Current beta boundaries

- Windows-only, single user.
- HEIC catalogue previews depend on available decoding; full-resolution HEIC edit/export remains disabled unless decoding succeeds.
- No conventional RAW development, face recognition, person identity/attribute inference, semantic search, video, full Lightroom catalogue compatibility, direct cloud APIs or cloud catalogue sync. Intelligent People masking is a combined semantic class, not instance or identity recognition. XMP is a conservative keyword/location/triage sidecar subset, not a general XMP editor.
- The 0.2 beta installer is unsigned and for named testers using disposable collection copies only.

See `BETA_TESTING.md`, `MILESTONE_3_VERIFICATION.md`, `MILESTONE_4_LOCATION_GUIDE.md`, `MILESTONE_5_INTEROPERABILITY_GUIDE.md`, `PRIVACY.md`, `THIRD_PARTY_NOTICES.md` and `RELEASE_GATES.md` before distributing a build.

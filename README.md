# Keepframe 0.2 beta

Keepframe is a local-first Windows photo catalogue for safely organising, triaging and mapping a personal archive, then preparing and tracking AI-assisted restorations. It is deliberately not a conventional RAW developer: protected originals, catalogue decisions and derived edits remain separate.

> Keepframe is a provisional beta name. Do not publish this build commercially until naming clearance and the release gates in `RELEASE_GATES.md` are complete.

## Principal workflow

1. Choose or reconnect a portable master-library folder.
2. Import with **Copy** (default) or explicitly choose **Move after verification**.
3. Browse the timeline, tags and OpenStreetMap-backed map; click the map to place an unlocated photograph.
4. Triage with `M` to Keep, `X` to Discard and the arrow keys to browse.
5. Move discarded photographs to reversible Keepframe Trash. Empty Trash is separately confirmed.
6. Create and edit a provider-specific recipe for local Qwen editing, ChatGPT or Gemini.
7. Export a full-resolution sRGB PNG for an external service and import the returned image as a traceable candidate version.

Catalogue, triage and manual external-edit workflows work without either optional AI service. Keepframe does not submit to ChatGPT or Gemini APIs and stores no cloud API keys.

## Safety model

- Copy and duplicate retention are always the import defaults.
- A staged file and final managed file must match the source SHA-256 before catalogue registration.
- Move removes a source only after managed placement, verification and catalogue commit, followed by an immediate source identity recheck.
- Unsupported, changed, missing or failed source files are retained.
- Discard is only a catalogue decision. Trash is internal and reversible; Empty Trash uses the Windows Recycle Bin.
- SQLite runs in WAL mode. Schema upgrades create and integrity-check a SQLite-consistent backup.
- Each library has a UUID marker and an exclusive lock. Missing/corrupt libraries open recovery rather than silent first-run setup.
- Browser media access is restricted to generated previews and derived edits inside the current library. RAW files are not served to the WebView.

## Clean-checkout validation

Requirements: Windows 10/11, WebView2, Node.js 24+, pnpm 11+, Rust 1.96+ and Python 3.11+.

```powershell
pnpm install
pnpm check:all
```

The command runs React tests, TypeScript/Vite production build, Rust tests, strict Clippy and the lightweight Python schema tests. It creates `ai-worker\.test-venv` with Pydantic only; PyTorch and model weights are not installed.

Run a disposable native profile:

```powershell
$env:KEEPFRAME_SETTINGS_DIR = "$PWD\.test-profile"
pnpm tauri dev
```

Build the invitation-only NSIS beta:

```powershell
pnpm tauri build --bundles nsis
```

## Optional local AI

The existing image-edit service defaults to `http://127.0.0.1:7868`. Keepframe rejects non-loopback service addresses. Optional Qwen3-VL analysis is installed separately; no model downloads automatically.

```powershell
.\scripts\setup-ai-worker.ps1
.\scripts\download-analysis-model.ps1
```

The runtime may require several GB; it is installed at `D:\AI Models\Keepframe\runtime`. The model download is approximately 17.5 GB and requires explicit confirmation. Every weight, cache and companion-worker asset is constrained beneath `D:\AI Models\Keepframe`.

When the vision model is absent, Keepframe now labels the recipe as a deterministic controls-only fallback. Qwen-Image-Edit at port 7868 is a separate integration and is reported independently in Settings & Health.

## Current beta boundaries

- Windows-only, single user.
- HEIC catalogue previews depend on available decoding; full-resolution HEIC edit/export remains disabled unless decoding succeeds.
- No exposure/curve/mask tools, conventional RAW development, face recognition, semantic search, video, XMP writing, direct cloud APIs or cloud catalogue sync.
- The 0.2 beta installer is unsigned and for named testers using disposable collection copies only.

See `BETA_TESTING.md`, `PRIVACY.md`, `THIRD_PARTY_NOTICES.md` and `RELEASE_GATES.md` before distributing a build.

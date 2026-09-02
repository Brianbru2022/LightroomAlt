# Keepframe

Keepframe is a private Windows photo catalogue and AI workshop. It moves imports into a managed master library after hash verification, keeps managed originals untouched, supports map/tag/timeline browsing and provides a three-state keyboard triage workflow. AI edits are provider-neutral recipes saved as derived versions.

## What works

- Tauri 2 desktop shell with React/TypeScript and Rust.
- First-run master-library selection and self-contained SQLite catalogue.
- Verified staging copies followed by source removal, SHA-256 duplicate detection and `Year/Month/Day` organisation.
- Bundled ExifTool 13.59 metadata and RAW-preview extraction.
- JPEG, PNG, TIFF, HEIC and common RAW discovery; unsupported/corrupt files are reported per import.
- RAW+JPEG logical pairing, thumbnails, timeline, tags, GPS map and manual placement.
- Keep/Undecided/Discard decisions, keyboard shortcuts and persistent undo history.
- Versioned edit recipes and deterministic ChatGPT, Gemini and local-Qwen prompts.
- Persistent batches/jobs, restart recovery and one-at-a-time local calls to `http://127.0.0.1:7868/api/edit-image`.
- Optional authenticated Qwen3-VL-8B analysis worker with deterministic fallback.

Discard never deletes or moves a photograph. Direct cloud API submission, video, ratings, face recognition, semantic search and XMP writing are intentionally absent from v1.

## Development

Requirements are Node.js 24+, pnpm 11+, Rust 1.96+ and the Windows WebView2 runtime.

```powershell
pnpm install
pnpm test
pnpm build
cargo test --manifest-path .\src-tauri\Cargo.toml
pnpm tauri dev
```

If a managed environment blocks package postinstall scripts, run TypeScript and Vite through their local Node entrypoints:

```powershell
node .\node_modules\typescript\bin\tsc -b
node .\node_modules\vite\bin\vite.js build
```

## Local AI

Keepframe uses the existing local edit service at `http://127.0.0.1:7868`. Catalogue functions remain available if that service is stopped.

Image-specific Qwen analysis is optional. Its runtime and model are installed separately:

```powershell
.\scripts\setup-ai-worker.ps1
.\scripts\download-analysis-model.ps1
```

The second script states the expected 17.5 GB download and requires the exact confirmation `DOWNLOAD`. All weights and caches are constrained to `D:\AI Models\Keepframe`. Without the model, Keepframe creates a safe deterministic recipe from the selected intent, metadata and preservation controls.

## Library safety

- Sources are read and copied, never removed.
- A staged copy must match the source SHA-256 before it is moved into `Originals`; the external source file is removed only after catalogue registration succeeds.
- Catalogue metadata is stored in `.keepframe\catalogue.sqlite` using WAL mode.
- Existing catalogues are backed up before schema migrations.
- Thumbnail caches can be deleted and regenerated without catalogue loss.
- A stale running AI job is restored to `queued` when Keepframe starts again.

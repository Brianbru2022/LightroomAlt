# Third-party notices

Keepframe includes or depends on open-source software. Release packages must retain the complete licence texts shipped with each component and the package-manager lockfiles.

- Tauri and Tauri plugins — Apache-2.0 / MIT.
- React, TypeScript and Vite — their respective open-source licences.
- SQLite through rusqlite — SQLite public-domain terms; rusqlite MIT.
- ExifTool — Perl Artistic License / GPL terms as distributed with the bundled ExifTool package.
- LibRaw and `dcraw_emu` — LGPL 2.1 or CDDL 1.0; bundled texts are under `src-tauri/resources/libraw/libraw-0.22.2-win64/`.
- Leaflet and OpenStreetMap — Leaflet BSD-2-Clause; map data © OpenStreetMap contributors under ODbL. Attribution remains visible in Map.
- Qwen models and any separate local image-edit service — not bundled. Their own model/software licences apply when installed by the user.
- Microsoft BEiT base fine-tuned on ADE20K (`microsoft/beit-base-finetuned-ade-640-640`) — optional, not bundled, Apache-2.0 model licence at the pinned source revision. Its separately installed Python path uses PyTorch, Transformers, Pillow, FastAPI and uvicorn under their respective open-source licences.
- Google SigLIP base patch16/224 (`google/siglip-base-patch16-224`) — optional, not bundled, Apache-2.0 at pinned revision `7fd15f0689c79d79e38b1c2e2e2370a7bf2761ed`. Its local provider uses the same separately installed PyTorch, Transformers, Pillow, FastAPI and uvicorn runtime.
- Lensfun calibration data — a deliberately small embedded subset under CC-BY-SA-3.0, sourced from the Lensfun data repository at pinned revision `12f5976ce30c024f98c420835125b9676ac07811`. The exact source XML filenames and coefficients are retained in `src-tauri/src/advanced_develop.rs`; no proprietary Adobe profiles are included.
- SCUNet colour real PSNR weights — optional, not bundled, Apache-2.0, distributed through the upstream KAIR v1.0 release and pinned by exact byte size and SHA-256. Runtime loading uses the separately installed Spandrel and PyTorch packages.
- Real-ESRGAN x4plus weights — optional, not bundled, BSD-3-Clause, distributed through the upstream Real-ESRGAN v0.1.0 release and pinned by exact byte size and SHA-256. Runtime loading uses the separately installed Spandrel and PyTorch packages.

This summary is not a substitute for the full licence files. Before any external installer is distributed, generate a dependency licence inventory, include all required notices and have the result reviewed.

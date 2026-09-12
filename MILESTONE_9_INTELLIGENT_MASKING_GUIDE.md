# Milestone 9: intelligent masking

## Scope and architecture

Intelligent masking is an optional, local mask-creation facility. It does not create a second editing or rendering system. The Develop UI calls a provider-shaped Tauri boundary for health, explicit installation, cancellation and prediction. Tauri sends a 640-pixel-bounded PNG to an authenticated worker on an ephemeral `127.0.0.1` port. The worker returns a compressed greyscale coverage PNG plus measurements and provenance. No cloud fallback exists.

A prediction is temporary. It appears as an overlay and changes no recipe, history or catalogue row until **Accept**. Cancel, another request, an asset change, a catalogue change or leaving Develop invalidates the generation token. The underlying PyTorch call cannot be interrupted safely once executing, so inference cancellation is logical: a late completion is discarded. Installation cancellation is physical between response chunks and removes the current `.partial` file.

The worker is lazy-loaded and warm-reused. It selects CUDA when PyTorch reports a usable CUDA device, falls back to CPU if device placement fails, and retries once on CUDA out-of-memory using CPU. `KEEPFRAME_SEGMENTATION_DEVICE=cpu` forces CPU for diagnosis and qualification. DirectML was assessed but deferred: the existing maintained worker already uses PyTorch, CUDA is available through that runtime, CPU provides the broad Windows path, and adding a second ONNX execution stack would increase package and conversion risk for this milestone.

## Model decision and licensing

| Candidate | Decision | Licence evidence | Practical assessment |
|---|---|---|---|
| `microsoft/beit-base-finetuned-ade-640-640` | Adopted | The pinned Hugging Face model card declares Apache-2.0; Microsoft UNILM source is MIT. | One approximately 900 MB semantic model supplies actual Subject-derived object classes, combined People and Sky at a 640-classification scale. No instance segmentation. |
| U2-Net | Rejected for the combined feature | Official repository is Apache-2.0. | Compact/full salient-object models are attractive for Subject but do not supply semantic People and Sky classes. |
| MODNet | Rejected for the combined feature | Official repository states code, models and demos are Apache-2.0. | Strongly specialised for human matting; People only, not general Subject or Sky. |
| CLIPSeg | Rejected by licensing gate | Repository code is MIT but explicitly excludes model weights from that licence. | Text-prompt flexibility is useful, but weight redistribution/use terms are not clear enough for adoption. |
| SkyAR | Rejected by licensing gate | Official repository is CC BY-NC-SA 4.0. | Sky-specific, non-commercial terms are unsuitable for an intended distributable application. |
| BiRefNet | Deferred | Pinned Hugging Face repository declares MIT and is approximately 444 MB. | Good general foreground candidate, but it remains Subject-only and would require another model for People/Sky. |

Selected model details:

- model: `microsoft/beit-base-finetuned-ade-640-640`;
- revision: `a8b6f5ef4acb2ea55d882989deaa02d39401e2b2`;
- weight: `pytorch_model.bin`, 899,902,905 bytes;
- weight SHA-256: `e0747360d190bd7c0f53d2fe3b2ed560c304d3eefef94574af9c7e93aaf8e7a9`;
- model source: <https://huggingface.co/microsoft/beit-base-finetuned-ade-640-640/tree/a8b6f5ef4acb2ea55d882989deaa02d39401e2b2>;
- model licence: Apache-2.0;
- runtime: PyTorch (BSD-style), Transformers (Apache-2.0), Pillow (HPND), FastAPI/uvicorn;
- storage: `D:\AI Models\Keepframe\segmentation\beit-base-ade20k-640`.

The exact three files are downloaded only after an explicit UI action or `DOWNLOAD` confirmation in `scripts\download-segmentation-model.ps1`. Downloads use `.partial` files, exact expected byte lengths and the pinned weight hash. A matching `MODEL_SHA256.txt` marker is required, and the worker hashes the full weight file once per process before loading. Model files are never placed in a catalogue, project export, XMP or photograph metadata.

## Capabilities and compromises

- **Subject** chooses one credible, centrally weighted model-labelled object class from a conservative allow-list. It is not a generic saliency/matting model and can miss unusual objects, small subjects or multiple subjects of different classes. Instances of the chosen class may be combined.
- **People** combines every pixel classified as the ADE20K `person` class. It deliberately offers one **People** mask; it does not pretend to provide person instances and performs no identity or attribute inference.
- **Sky** uses the model's `sky` class. This is real semantic output, never a brightness heuristic. Fine branches, hair, glass, distant horizons and rare lighting can remain imperfect.
- A region below 0.2% of the analysis image or below 30% mean selected-class confidence returns a clean no-result. Sky predictions covering more than 95% are also rejected: qualification showed that the scene model could otherwise label a featureless bright wall as whole-frame sky.

Logits are bilinearly resized to the analysis image. A 3×3 median pass removes isolated class noise and a conservative 0.7-pixel Gaussian pass reduces jagged probability edges. Accepted masks use Catmull-Rom coverage resizing in the existing renderer. The ordinary Mask feather control is separate and remains at zero by default, so acceptance does not indiscriminately soften every edge.

## Accepted data, schema and portability

Recipe schema remains version 2 and catalogue schema remains version 9. The existing tagged mask geometry gains `kind: "semantic"` with:

- analysis width and height in the same EXIF-oriented, uncropped canonical basis as Milestone 8;
- base64-encoded compressed greyscale PNG coverage;
- SHA-256 checksum;
- provider/model/category/execution provenance;
- normalised Add/Subtract brush refinement strokes.

The accepted payload is authoritative. Rendering validates and decodes it, then uses it without any worker, cache or model access. Removing model weights, deleting previews/caches or upgrading the provider cannot change accepted pixels. New model versions affect only new predictions or explicit Re-detect proposals.

No database migration is required because recipe v2 already owns tagged mask geometry. Older v1/v2 recipes and manual masks retain their existing interpretation. Portable catalogue JSON advances from format schema 1 to 2 and embeds each asset's validated Develop recipe, including compressed accepted coverage. The model itself is never exported. XMP remains metadata-only and does not contain masks.

Copy with Masks copies the accepted coverage and refinements, assigns fresh mask IDs and never reruns inference. Enable/disable, rename, opacity, feather, invert, duplicate, local controls, solo preview, undo/redo, preview cache identity and full-resolution export use the existing mask pipeline.

## Qualification record

The closure run used two newly generated, non-private 640×427 PNGs outside the repository: an outdoor golden-hour scene containing two adults, a dog, a car, mountains and fine branches, and a bright interior with no people and overcast sky visible through windows. A featureless 640×427 wall fixture tested false positives. The machine has an NVIDIA GeForce RTX 5090; the qualification runtime used PyTorch 2.13.0+cu130 and Transformers 5.14.1. These figures are measurements from that machine, not general performance promises.

| Measurement | CUDA | CPU |
|---|---:|---:|
| First request wall time, including full weight hash/import/load | 7,749–18,422 ms across two fresh processes | 11,861 ms |
| Weight construction/device placement, excluding the integrity hash and imports | 600–631 ms | 190 ms |
| Representative warm preprocessing | 3–8 ms | 63 ms |
| Representative warm inference | 52–61 ms | 2,661 ms |
| Representative post-processing | 39–48 ms | 332 ms |
| Process resident memory after the run | approximately 1,547 MiB | approximately 1,584 MiB |
| CUDA allocated after the run | approximately 657 MiB | not applicable |
| CUDA peak allocated during inference | approximately 1,751 MiB | not applicable |
| CUDA reserved after the run | approximately 2,646 MiB | not applicable |

The outdoor result selected the car as Subject, combined both adults into one honest People mask, and separated the blue/cloud/golden sky around the mountain, trees and branches. Boundaries were usable but visibly coarser than dedicated matting, particularly around hair and fine detail. The interior returned a furniture Subject, correctly returned no People, but missed the overcast sky through the windows. The blank fixture returned no Subject and no People. It initially exposed the whole-wall Sky false positive; the greater-than-95% credibility guard was added and the real model then returned a clean no-result.

A deterministic 640×427 accepted-mask fixture measured 149.9 ms to decode and Catmull-Rom rasterise coverage at 720×480, 0.040 ms for the SQLite payload insert, and 403.0 ms for the accepted one-mask 720×480 preview render. Its compressed fixture PNG was 2,523 bytes, catalogue JSON was 4,640 bytes and the temporary uncompressed analysis coverage was 273,280 bytes. Real qualification masks ranged from 5,063 to 14,097 compressed bytes. Browser overlay, Accept, Cancel and refinement rendering were also inspected, but native WebView compositing was not separately instrumented.

CI deliberately exercises deterministic semantic fixtures and mocked provider responses without downloading weights. Real-model qualification is optional and remains outside normal CI.

## Residual manual smoke

Use a disposable catalogue and one legally approved JPEG plus one supported RAW:

1. verify/install the segmentation model;
2. create Subject and Cancel, confirming no recipe change;
3. create Subject again and Accept;
4. change local exposure, Add Brush and Subtract Brush;
5. create and accept Sky and People masks;
6. restart and verify every accepted mask;
7. temporarily rename/disable the model directory and verify accepted masks still render;
8. crop, rotate, straighten and flip, checking alignment;
9. export full resolution and compare with Develop;
10. Copy with Masks to another asset;
11. verify the protected original SHA-256 is unchanged.

Native WebView click-through and real camera-specific RAW acceptance remain separate from browser rendering and automated deterministic tests.

# Milestone 14 — Local Semantic Discovery Guide

## Safety, privacy and scope

Semantic intelligence is advisory, derived and optional. Photographs, queries and embeddings stay on the local computer; no telemetry, cloud inference, remote vector store, face recognition or identity recognition was added. Removing the model or deleting the semantic tables disables discovery but leaves Library browsing, Develop, export, metadata, Collections, stacks and versions usable. No suggestion writes ratings, flags, keywords or organisation. A normal Stack or Smart Collection is created only after the user explicitly accepts the visible proposal.

Ordinary visible-object and scene queries, including people generally, are supported by the image/text model. Keyword generation is deferred: SigLIP is a retrieval model rather than a calibrated multi-label classifier, and this milestone does not create sensitive-attribute labels or infer exact locations from pixels. Existing metadata location filters remain deterministic.

## Model qualification and licence gate

| Candidate | Practical finding | Decision |
| --- | --- | --- |
| `google/siglip-base-patch16-224` | Shared image/text space, 224 px input, 768-float vectors, maintained Transformers implementation, manageable desktop weight, CPU and CUDA paths, Apache-2.0 model card | Selected |
| `openai/clip-vit-base-patch32` | Compact and widely supported, but the inspected Hugging Face model metadata did not provide a sufficiently explicit weights licence for this redistribution gate | Rejected for this integration |
| Apple MobileCLIP | Attractive compact models, but published weights use Apple ML Research Model Terms while repository code is MIT; the weights terms are not the clean permissive redistribution fit required here | Rejected |

The selected identity is `google/siglip-base-patch16-224` revision `7fd15f0689c79d79e38b1c2e2e2370a7bf2761ed`, Apache-2.0, from the pinned Hugging Face revision. `model.safetensors` is 812,672,320 bytes with SHA-256 `421489ed67220ff3cf58fefe883271f5dd6fd1bca1f2d24157453b5cc8f82f88`; the seven required downloaded files total 815,871,927 bytes (about 778 MiB). Input is 224×224 and output is a normalised 768-dimensional float vector.

The app never silently downloads it. The install panel shows model, licence, source, size and local path, asks for explicit confirmation, reports per-file/total progress, supports cancellation, validates exact file lengths, validates JSON, verifies the weight and SentencePiece SHA-256 values and stages each download as `.partial`. Model assets live at `D:\AI Models\Keepframe\semantic\siglip-base-patch16-224`; weights never enter a catalogue, portable export, original folder, XMP or Collection.

## Provider and runtime

React calls typed Tauri commands and has no Python/model dependency. The Rust semantic boundary owns provider metadata, provenance validation, cancellation generations, catalogue queries and storage. The existing app-owned loopback worker exposes authenticated image- and text-embedding endpoints. Its SigLIP session is lazy and warm, uses `local_files_only=True` plus offline Hugging Face/Transformers flags, serialises inference through a lock, and unloads with the existing worker lifecycle.

PyTorch/Transformers first uses CUDA when available and otherwise CPU. A CUDA out-of-memory error unloads and retries that image once on CPU; device status and timing fields are returned. DirectML/ONNX is deferred because it would add a second runtime and conversion/qualification path. A shared Rust GPU gate is released after every image and a 25 ms inter-item delay gives interactive Develop work a scheduling opportunity. The implementation uses one-image batches to bound RAM/VRAM. Actual single-versus-small-batch throughput remains part of real-model acceptance because the optional weights were not downloaded during this closure.

## Catalogue schema, identity and portability

Catalogue schema v13 adds three isolated tables:

- `semantic_embeddings`: one binary vector and 64-bit perceptual hash per physical `source_id`, keyed by model, exact revision and preprocessing version, with the protected-source SHA-256 and indexed timestamp;
- `semantic_index_queue`: a unique persistent source-level work item with priority, state, attempts and bounded error text;
- `semantic_suggestion_decisions`: lightweight explicit Accept/Dismiss decisions keyed by a stable hash of kind, current model revision and sorted source IDs.

Import/representation triggers deduplicate and enqueue new sources. Selected/visible IDs can raise queue priority. A model/preprocessing mismatch queues only missing/currently incompatible sources. A representation hash change requeues the source; a rescan that marks protected content modified deletes its derived embedding and records a failed item until the source is resolved. Missing files fail safely. A hash-confirmed relink that restores the same content does not invalidate a valid vector. Running queue rows are returned to queued state after restart; individual decode/inference failures become isolated failed rows. Pause/cancel occurs between safely committed source rows.

The image embedding represents protected source appearance, using Keepframe's immutable source-derived preview as model input; virtual Develop appearances are deliberately deferred. Sibling versions share one embedding, but result selection uses the first version that satisfies deterministic item filters. One physical source appears once by default, so sibling versions cannot flood results.

Portable catalogue schema v4 carries only explicit semantic suggestion decisions in addition to Milestone 13 organisation. Embeddings, pHashes, queue state, scores, models and indexes remain excluded. Import validates decisions, restores them transactionally, queues missing local embeddings through normal schema triggers/status reconciliation, and leaves Collections, stacks, versions and the Library usable before reindexing.

## Search and exploration

The collapsible Library **Local Discovery** surface has Search, Similar, Duplicates, Suggestions and Smart proposal sections. Semantic search embeds up to 240 characters locally, performs cosine retrieval over compatible source vectors and sends ordinary catalogue item IDs back through the existing Grid. Grid cards, selection, rating/flag controls, context actions, Compare and Survey remain the established Milestone 12/13 surfaces.

Ranking is explicit and testable: by default 85% normalised source-image cosine similarity plus 15% exact filename/title/caption/keyword term coverage. Current rating, flag, file type, edited, Collection and date filters are applied deterministically before top-K. The default top-K is 80 and backend input is capped at 240. Ties use stable source-ID order. Labels are **Strong match**, **Moderate match** and **Possible match**; displayed numbers are similarity scores, never probabilities or confidence claims. Explanations are fixed factual signals rather than generated prose.

**Find Similar** uses the selected source vector, cosine similarity and the same one-source result policy. Duplicate discovery remains separate: exact groups use the existing protected-source SHA-256 identity; near duplicates require both dHash Hamming distance ≤6 and image similarity ≥0.985. Four 16-bit dHash projections create locality buckets before comparisons, avoiding a catalogue-wide quadratic pass. Burst proposals require adjacent capture times within three seconds and image similarity ≥0.94. Broad semantic similarity alone cannot create a duplicate/burst proposal. Groups open in Compare for two items or Survey for larger groups; Dismiss persists, while Accept explicitly creates a normal Milestone 13 Stack. No automatic deletion, merging, rejecting, rating, preferred-shot claim or collapse exists.

Natural-language Smart assistance uses a conservative deterministic parser, not a generative model or SQL. It emits only existing allow-listed `SmartRule` objects for clear rating, flag, edited, version status, camera and year/date patterns. Unsupported text fails closed. The proposed fields/operators/values are shown and editable, and nothing is persisted until **Save Smart Collection explicitly**. Semantic Smart rules, AI Collection auto-creation and search history are deferred.

## Index status, health and rebuild

Library and Settings show the concise model/index state, indexed/total/queued/failed/stale counts, provider, revision, dimensions, storage and execution provider. **Rebuild Semantic Index** confirms intent, deletes only derived embeddings/queue rows, then repopulates the incremental queue. Catalogue Health reports stale model revisions, absent/current rows through queue state, orphan rows, wrong dimensions/vector lengths, corrupt perceptual hashes and source-hash mismatches. Repairs rebuild derived rows and never touch photographs or organisation.

Binary float32 storage is 3,072 vector bytes plus 8 pHash bytes per source before SQLite row/index overhead: about 30.8 MB payload for 10,000 sources and 308 MB for 100,000. Brute-force cosine is deliberately used at this scale; it is simple, deterministic and persisted entirely in SQLite. A future approximate index can be added behind the provider/storage boundary for materially larger catalogues without changing source identity or portable format.

## Performance and qualification status

`scripts/benchmark-semantic-discovery.ps1` creates 20,000 source vectors in a transaction and measures actual SQLite decode/filter/cosine/top-K retrieval in a release build. The closure run built the corpus in 195 ms, retrieved top-80 in 111 ms at 10,000 and 312 ms at 20,000, completed Find Similar at 20,000 in 277 ms, and completed the bucketed 20,000-source duplicate scan in 279 ms. The binary vector+pHash payload was 61,600,000 bytes. Existing renderer, export, Library and organisation benchmark scripts remain the regression authority.

Cold model load, warm text/image inference, representative JPEG preprocessing, end-to-end indexing throughput, process RAM/VRAM and manual ranking quality are not fabricated: the optional 778 MiB model was not installed in this checkout. The worker returns separate load/preprocess/inference timings when run, and each index progress event carries those timings. A legal/non-private qualification set should cover dog, beach, car, mountain, people, sunset, building, night, food and forest plus same-scene crops, bursts, unrelated same-colour scenes, resized copies and exposure variants. Native/real-model observations remain external verification.

Known deliberate limits: one-image inference batches; source appearance only; brute-force search loads compatible vectors for a query; no semantic Smart rule, keyword labelling, face/identity recognition, preferred-shot ranking or exact pixel-geolocation. Missing sources can appear from retained embeddings and are labelled Missing, but cannot be re-embedded until relinked. Browser fixtures verify workflow/state contracts, not model quality.

## Disposable native acceptance checklist

1. Install and verify the pinned model from the explicit prompt.
2. Index a disposable catalogue containing at least 100 legal/non-private images.
3. Pause indexing.
4. Resume indexing.
5. Close and reopen Keepframe.
6. Verify queue progress persists and running work safely resumes.
7. Search `dog`.
8. Search `sunset`.
9. Search `building at night`.
10. Combine semantic search with the 4+ stars filter.
11. Use Find Similar from a selected photograph.
12. Inspect sibling-version collapse and version-specific filters.
13. Inspect a retained result whose source is Missing.
14. Create/locate a conservative near-duplicate candidate group.
15. Dismiss that group.
16. Restart and verify the dismissal remains authoritative.
17. Accept a burst suggestion into a normal Stack.
18. Undo/remove the Stack through the normal organisation workflow.
19. Generate a Smart Collection proposal from plain English.
20. Inspect every proposed rule.
21. Edit a proposed rule.
22. Save explicitly.
23. Change metadata and verify live Smart membership.
24. Import new photographs.
25. Verify only new/missing embeddings are queued.
26. Relink a Missing source with an exact hash match.
27. Verify a matching embedding is retained or an invalid one is rebuilt appropriately.
28. Rebuild the Semantic Index.
29. Verify Library navigation remains usable during rebuild.
30. Use Develop during background indexing and confirm it remains responsive.
31. Observe CPU, GPU, RAM and VRAM during cold load and indexing.
32. Verify no photograph/query/inference traffic leaves the machine.
33. Export portable catalogue schema v4.
34. Import it into a clean/disposable copy of the same physical library.
35. Verify versions, Collections, stacks and dismissals work before reindexing.
36. Rebuild and verify semantic search returns.
37. Hash protected originals before and after the workflow.
38. Confirm no source file changed.

# Milestone 13 — Catalogue Organisation Guide

## Identity and schema

Catalogue schema v12 separates a physical `sources` row from Library-visible `assets` (catalogue items). `representations.source_id` owns the protected file path, SHA-256, size and file type. Source facts—filename, capture time, dimensions, camera/lens, GPS, availability and the immutable source-preview path—live in `sources`. No virtual-version operation creates or copies a representation.

Every source has exactly one primary catalogue item, enforced by a partial unique index. Existing item IDs become primary item IDs during migration, preserving recipes, ratings, flags, titles, captions, keywords and appearance. Each sibling item has a stable ID, `source_id`, `version_index`, optional display name and its own metadata/recipe rows. A `(source_id, version_index)` index provides deterministic sibling order.

Portable catalogue schema v3 serialises source relationships, catalogue items, version names and recipes, Collection Sets, Collections and Smart rules, item memberships, Stacks/member order/top, and sibling-collapse state. It excludes thumbnails, caches, models and transient selection. Import applies only to a library containing the matching physical source identities, validates the complete payload first, creates a verified SQLite backup, then restores catalogue-native state in one transaction. It never writes or removes image pixels.

## Metadata policy

| Source-shared | Catalogue-item/version-specific |
| --- | --- |
| protected path and SHA-256 (via representations) | Develop recipe and edited state |
| file type and dimensions | rating and Pick/Reject flag |
| capture time and camera/lens | version name |
| embedded/manual GPS and Missing state | editable title, caption, creator and copyright |
| immutable source preview | keywords and Collection membership |

Keywords remain item-specific because the existing catalogue already attaches keywords to item IDs; this enables a deliverable version to carry different workflow keywords without rewriting EXIF/XMP. Source-derived filtering such as camera, lens, capture date and file type naturally matches every sibling through `sources`.

## Virtual versions

- **From current** copies the accepted Develop recipe but starts fresh rating, flag, title, caption and keywords.
- **From original** starts with the neutral recipe and fresh workflow metadata.
- **Duplicate** copies recipe and mutable metadata, assigning a fresh item ID.
- Rename changes display text only. Identity never depends on a name and names need not be globally unique.
- Reset, Auto, presets, Copy/Paste and Sync continue to target selected item IDs. Siblings change only when explicitly selected.
- **Delete Version** is available only for non-primary items. It cascades catalogue-only recipe, membership and derived-thumbnail state; it cannot enter the filesystem Trash path. A Stack Top must first be changed or unstacked.
- Source removal remains the existing primary/source-wide Trash workflow. The backend rejects a virtual item passed to physical Trash and tells the caller to use Delete Version.

Grid thumbnails are regenerable recipe renders stored under `.keepframe/previews`. The immutable source thumbnail remains on `sources`, so sibling previews do not become inputs to one another. Milestone 10 decode keys remain path/metadata based and can be shared; downstream intermediate keys contain recipe dependencies, preventing cross-version output reuse.

Compare and Survey already operate on selected item IDs, so Primary vs Version, sibling vs sibling and unrelated versions use the same zoom/pan and rating/flag tools. Grid, Filmstrip and Develop show version identity; the Library organiser creates versions and expands/collapses sibling groups.

Export loads the selected item’s recipe and rating with its shared physical source. `{version}` is a validated filename token, using the version name or deterministic `Primary`/`Version N` fallback. Existing collision Ask/Skip/Replace/Unique behaviour remains authoritative if a template omits the token.

## Collections and Smart Collections

Manual Collection membership references catalogue item IDs, not sources. Adding a representative or virtual version adds only that selected item. Deleting a Collection or Collection Set removes organisation metadata only. Collection Sets are deliberately one level deep; Collections have stable `position` order. Image ordering uses existing deterministic catalogue sorts rather than a new drag-order system.

Smart Collections compile validated rules directly to parameterised SQLite predicates. No result membership is persisted. Supported fields are Rating, Flag, Edited, File Type, Keyword, Capture Date, Import Date, Camera, Lens, Primary/Virtual status, multiple-version status, and stacked/unstacked status. Operators are limited by type: equals/GTE/LTE, is/contains/does-not-contain, before/after/between, and boolean/state `is`. Match ALL and Match ANY are supported. Catalogue changes therefore affect results on the next query without a rebuild. Counts are evaluated when the organiser refreshes, not on every rendered frame.

## Stacks and selection

Manual Stacks contain ordered catalogue item IDs and have an explicit top item. An item can belong to only one Stack. Create, add, remove, collapse, expand, change Top and Unstack are transactional. Removing a two-item member dissolves the Stack safely. Arbitrary member drag-reordering is deferred; persisted insertion order and Set as Top are supported.

Collapsed queries return only the top item and its count. Selecting it selects that item only. Normal ratings, flags, metadata, presets, Auto and export never silently expand to hidden members. Expanded members are independently selectable. Switching Collections or collapsing a Stack retains the global selected-ID set; hidden selections are counted separately and reappear when their context returns. Shift-range always uses the current visible query order.

Sibling grouping and manual Stacks are separate concepts. A version can participate in a Stack, but creating a virtual version does not create a manual Stack.

## Filesystem, XMP and health boundaries

Import duplicate detection and folder watching continue to enumerate `representations`, so virtual items are not interpreted as files. Rescan runs once per source, propagates Missing/available state to all siblings, and inspects item-owned derived renders separately. Exact hash relink resolves `representations.source_id` and restores every sibling at once.

There is one conventional XMP sidecar per source. Existing primary-item explicit export/import semantics remain; a virtual item is rejected for XMP rather than competing for that path. Virtual recipes remain catalogue-native.

Catalogue Health now checks SQLite integrity plus exactly one primary per source, orphan Collection/Stack membership, duplicate Stack membership, validated Smart rules and valid version recipes. It reports corruption and does not delete organisation data as an automatic repair.

## Performance and limitations

The Milestone 13 benchmark creates 10,000 sources and 20,000 items, opens a 10,000-member manual Collection, and evaluates multi-rule ALL/ANY Smart Collections. The closure run measured 245 ms fixture construction, 114 ms to create/open the 10,000-member Collection, 5 ms for Smart ALL and 6 ms for Smart ANY. Run `scripts/benchmark-catalogue-organisation.ps1`.

The opt-in regression runs also passed: the Milestone 10 standard render measured 68.57 ms cold/1.201 ms warm (the 2400×1600 render was 284.52 ms plus 754.68 ms PNG encoding); the Milestone 11 ten-item full-resolution export completed with a 51,840,000-byte peak tracked RGB scope and 11,086 ms average, with the individual full JPEG path faster than the retained prior artefact but whole-batch timing subject to encoder and machine-load variance; the Milestone 12 10,000-record run measured 116 ms filtering, 20 ms sorting, 0.496 ms select-all and 0.522 ms 9,000-item range selection in the final gate. Organisation state keeps IDs and lightweight JSON/metadata only; native working-set measurement remains external.

Known deliberate limits: Collection Sets are one level; collection and stack drag ordering are deferred; the global undo log remains available for metadata and recipe batches, while organisation commands rely on atomic transactions, explicit confirmations and portable/import backups rather than being added to the existing narrow undo decoder. Portable import targets the same physical library, not a photo-ingestion replacement. Native RAW, filesystem, memory and accessibility acceptance remain manual.

## Disposable native acceptance checklist

1. Open a real catalogue.
2. Create a version from the current edit.
3. Create a default version.
4. Rename both versions.
5. Edit each differently.
6. Restart Keepframe.
7. Verify every version and recipe persists.
8. Compare sibling versions.
9. Export both versions with `{version}`.
10. Verify separate outputs and correct pixels.
11. Create a manual Collection.
12. Add individual versions.
13. Remove one version.
14. Delete the Collection.
15. Verify all catalogue items and source files remain.
16. Create a Smart Collection for rating 4 or higher.
17. Change a version rating.
18. Verify Smart membership updates.
19. Create a multi-rule Match ALL and Match ANY Smart Collection.
20. Create a manual Stack from selected items.
21. Collapse it and verify only the Top appears.
22. Expand it and select members independently.
23. Change the Stack Top.
24. Unstack.
25. Verify every source and version remains.
26. Make one physical source unavailable.
27. Verify all its sibling versions show Missing.
28. Relink one exact hash match.
29. Verify all sibling versions recover.
30. Copy and Sync edits between versions.
31. Run Batch Auto on selected versions and inspect independent results.
32. Batch-export selected versions.
33. Export portable catalogue schema v3, then import it into a disposable copy of the same physical library.
34. Verify version names/recipes, Collections/rules/Sets and Stack order/top survive.
35. Recalculate and inspect protected source hashes.
36. Verify version, Collection and Stack operations did not alter originals.

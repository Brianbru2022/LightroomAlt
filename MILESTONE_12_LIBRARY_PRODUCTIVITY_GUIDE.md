# Milestone 12 — Library productivity

## Selection and active photograph

Keepframe stores selection as catalogue IDs: an active ID, an anchor ID and a set of selected IDs. Image records and pixels are not copied into selection state. A plain click replaces the selection, Ctrl-click toggles membership, Shift-click selects the inclusive range in the current filtered/sorted result order, Ctrl+Shift-click adds that range, Ctrl+A selects every ID in the current result and Ctrl+Shift+A or Escape clears it.

The active photograph is the strong outlined item. It drives Loupe, the Develop editor and the source for explicit Sync. Other selected photographs have a check and lighter outline. Selection survives layout changes, Develop, export and filter/sort changes. IDs hidden by a filter remain selected; the status reports total selected and the selected subset currently loaded/visible. A replacing click deliberately drops hidden selection.

## Keyboard culling

| Shortcut | Library action |
| --- | --- |
| Left / Right / Up / Down | Move the active selection in visible order |
| 0–5 | Apply a star rating to the selection |
| P or M | Pick |
| X | Reject — never deletion |
| U | Unflag |
| G | Grid |
| Space | Loupe |
| C | Compare |
| N | Survey |
| D or Enter | Develop the active photograph |
| Ctrl+A | Select all current results |
| Ctrl+Shift+A / Escape | Clear selection |
| Ctrl+Z / Ctrl+Shift+Z | Catalogue undo / redo |

Shortcuts are disabled while an input, textarea, select or editable control has focus. Delete and Backspace are not bound to catalogue removal. Auto Advance is an explicit, persisted toggle and advances after rating/Pick/Reject only when reviewing a single selection; it does not silently dismantle a multi-selection.

## Grid, Loupe, Compare, Survey and Filmstrip

The existing virtualised Grid still renders only a viewport plus bounded overscan. Loupe uses the catalogue preview, has review controls and shares the filtered-result Filmstrip. Compare starts with the active photograph as Reference and another selected photograph as Candidate. Candidates can be cycled, swapped and reviewed. Fit/100%, independent zoom and independent pan are available; Synchronise applies matching relative zoom/pan to both images and can be disabled for dissimilar aspect ratios.

Survey uses previews, never originals or full-resolution export inputs. It displays up to 12 loaded selected photographs in an adaptive grid, makes the active item explicit and allows items to be removed from the current selection. Larger selections are deliberately bounded to the first 12 loaded selected items.

The Filmstrip reflects the loaded portion of the current filtered/sorted result, marks both active and selected states, shows rating/flag state and scrolls the active item into view. Ctrl/Shift selection semantics are the same as Grid. Develop retains the full selection and includes the same Filmstrip; only the active recipe is edited until Sync is explicitly requested.

## Metadata, ratings and flags

Catalogue schema v11 adds rating (0–5), title, caption, copyright and creator. These values are catalogue-owned and never rewrite the protected original. The metadata dialog displays `— Mixed —` for differing descriptive values and changes a mixed field only when its checkbox is enabled. Capture date, camera and GPS are excluded.

Keyword Add and Remove preserve unrelated keywords. Replace is an explicit separate choice. Rating, flag and metadata changes across a selection run in one immediate SQLite transaction and create one `batch_assets` audit entry. Undo restores every affected scalar and keyword set; redo reapplies the complete after-snapshot.

Reject means the catalogue flag `discard`. Removal from the catalogue and deletion from disk remain outside this batch editor. Existing Keepframe Trash remains reversible; Empty Trash is separately confirmed and uses Windows Recycle Bin, so it is not claimed as an in-app undo.

## Develop Sync, copy/paste and presets

Sync always has an explicit source and destination set. Tone, White Balance, Presence and Colour default on. Crop/Transform, Manual Masks and Intelligent Masks default off. White balance is copied as the recipe's absolute light-balance and tint values; relative white balance is not invented.

Manual or intelligent mask sync copies the persisted definition, normalised geometry, accepted semantic coverage and provenance. Every destination receives fresh mask IDs. Semantic segmentation is not rerun. Each target's previous recipe is captured exactly, and the entire Sync is a single `batch_recipes` undo/redo unit. Copy settings records the active source; Paste settings opens the same category-safe Sync flow instead of copying asset metadata.

Built-in and user Develop presets can be applied to a selection. Only their declared global categories are merged; target masks and excluded settings remain unchanged. The batch is one undo/redo unit.

## Batch Auto and cancellation

Batch Auto first analyses every selected photograph independently with the deterministic Milestone 7 statistics. Its confirmation summarises selected, analysable, low-confidence white-balance and skipped/failed counts. Acceptance reruns independent bounded analysis sequentially, reports progress and preserves each asset's masks and geometry while changing its global Auto proposal.

Cancellation increments a generation token. Analysis already completed in memory is discarded and the transaction is not opened; therefore no partially accepted recipe batch is exposed. If analysis completes, successful recipes are committed together and failed assets are reported/skipped. One accepted Batch Auto is one undo/redo item.

## Filtering, sorting and output

Library filters now cover exact rating, flag, edited/unedited and reliable representation extension in addition to existing keyword, capture date, camera and location filters. Sorting supports capture time, import time, filename, rating and Develop edited time, ascending or descending. SQL order is shared by paged records and the all-result ID query, so Shift range never falls back to primary-key order.

Export selected passes the complete selected ID set into the Milestone 11 queue, including selected results not currently loaded as cards. Each asset still renders from its own protected source and accepted recipe.

## Performance and memory

Run:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\benchmark-library.ps1
```

The checkpoint creates 10,000 SQLite catalogue rows, measures filtered/sorted paging and a 1,000-asset transactional rating/keyword change. The frontend test separately measures construction of 10,000 selected IDs. Record the current-machine output with release evidence; timings are machine-specific. Grid DOM remains virtualised and Library/Survey/Compare use thumbnails or previews. Selection memory is O(number of IDs), not O(image pixels or asset records).

Measured on the current Windows development machine (debug/test builds):

| Operation | Time |
| --- | ---: |
| Select 100 IDs | 0.006 ms |
| Select 1,000 IDs | 0.037 ms |
| Select 10,000 IDs | 0.595 ms |
| Shift-select a 9,000-ID range | 0.520 ms |
| Filter 10,000 rows and return a 240-item page | 73 ms |
| Sort 10,000 rows and return a 240-item page | 59 ms |
| Rate 1,000 assets transactionally | 27 ms |
| Add a keyword to 1,000 assets transactionally | 35 ms |
| Sync Tone to 100 assets | 4 ms |
| Render Compare in jsdom | 6.547 ms |
| Switch the Compare candidate in jsdom | 14.150 ms |
| Render 12 Survey items in jsdom | 6.953 ms |

These are reproducible checkpoint measurements, not production guarantees. The browser runtime used for rendered verification did not expose a JS heap counter. Code inspection confirms selection retains only a set of IDs, Compare and Survey reuse bounded preview URLs, Batch Auto decodes one bounded thumbnail at a time, and Sync holds validated recipe snapshots rather than pixel buffers. Native working-set observation during real Batch Auto and a large Sync remains part of the external acceptance checklist.

## Native acceptance checklist

Use a disposable profile and a catalogue with at least 100 photographs:

1. Import or open a catalogue containing at least 100 images.
2. Select one photograph.
3. Ctrl-select several photographs.
4. Shift-select a range in the current visible sort order.
5. Select all current results, then clear the selection.
6. Apply a rating to the selection.
7. Apply Pick, Reject and Unflag; confirm Reject did not delete anything.
8. Enable Auto Advance and cull 20 photographs.
9. Open Compare.
10. Compare several similar photographs and cycle the Candidate.
11. Test synchronised and independent zoom/pan.
12. Open Survey with 6–12 photographs.
13. Remove photographs from Survey.
14. Add and remove batch keywords without disturbing unrelated keywords.
15. Edit mixed metadata, opting in only the intended fields.
16. Undo and redo the batch metadata operation.
17. Enter Develop with multiple photographs selected and confirm only the active recipe is edited.
18. Sync Tone only.
19. Undo the Sync and confirm every destination recipe is restored.
20. Explicitly Sync masks and confirm subsequent destination edits are independent.
21. Preview and apply Batch Auto to several photographs.
22. Start another Batch Auto and cancel it partway; confirm no partial batch is committed.
23. Apply a Develop preset to the selection and undo it.
24. Record that virtual-copy creation is not implemented in Milestone 12.
25. Record that version comparison is consequently not applicable.
26. Record that stacks are not implemented in Milestone 12.
27. Record that collections are not implemented in Milestone 12.
28. Filter Picks and confirm the selection policy is clear.
29. Filter Edited and test the relevant sort directions.
30. Batch export the current selection.
31. Verify exported filenames, collision handling and sequence ordering.
32. Hash or byte-compare representative originals before and after all operations.

## Deferred scope and known limits

- Virtual copies are deferred. The existing `versions` table represents derived pixel candidates, not independent catalogue identities; overloading it would make source identity and recipe ownership ambiguous.
- Stacks are deferred because no stable stack identity or collapsed-selection contract exists.
- Manual and Smart Collections are deferred because no collection schema exists; introducing one alongside first-class selection would expand the migration and query surface beyond this milestone.
- Colour labels are deferred because they did not previously exist and ratings/flags satisfy the core batch review workflow without another metadata enum.
- Manual drag ordering and automatic burst detection are deferred.
- Survey displays at most 12 loaded selected previews. The Filmstrip shows the loaded page and loads more near its end rather than creating 10,000 image elements.
- Native WebView click-through, a real 100+ photograph catalogue, camera-specific RAW files and installer acceptance remain manual/external verification gates.

# Keepframe interoperability and library resilience

Keepframe remains a local-first catalogue. Original image bytes are never changed by this feature set: metadata writes are explicit XMP sidecars, and all catalogue exports are local files chosen by the user.

## What the catalogue stores

| Portable photographic metadata | Keepframe catalogue state |
| --- | --- |
| Capture timestamp, keywords and slash-delimited keyword hierarchy, GPS coordinates | Three-state triage (`keep`, `undecided`, `discard`), asset identifier, manual-versus-embedded location provenance, managed paths, SHA-256 hashes, missing-state observations, preferred/derived versions and their local provenance |

Keepframe currently has no title, caption, star rating, colour label, or capture-date override field. It does not invent or export those fields.

## XMP sidecars

XMP writes are initiated from **Settings & Library Health** for a selected item, the active filter, or the full library. Whole-library export requires a confirmation because XMP may expose GPS information.

- The original is never overwritten; the sidecar is `<original-name>.xmp`.
- Existing sidecars are preserved unless the user explicitly chooses replacement.
- A replacement is written to a temporary file, XML-validated, synchronised, then promoted. The previous sidecar is restored if promotion fails.
- Exported portable fields are `dc:subject`, `lr:hierarchicalSubject`, `xmp:CreateDate`, `xmp:Label`, and `exif:GPSLatitude`/`exif:GPSLongitude` where present.
- The documented `https://keepframe.app/ns/1.0/` namespace records Keepframe triage, asset ID, location source, and manual/embedded coordinate provenance. It is used only because portable XMP alone cannot distinguish a user pin from embedded GPS.

XMP reading is intentionally conservative. Keepframe understands keyword bags, hierarchical keywords, GPS and its own triage/location attributes. It applies a field only when the catalogue has no value; different existing values are reported as conflicts. Unknown XMP is never rewritten by import. Malformed XMP is rejected for that operation and cannot prevent a library opening.

## Portable catalogue JSON

**Export catalogue JSON** creates `keepframe-portable-catalogue-v1.json` in a local folder you select. Its top-level fields are:

```json
{ "format": "keepframe-portable-catalogue", "schemaVersion": 1, "exportedAt": "...", "assets": [] }
```

Each asset records its ID, filename, triage, capture metadata, tags, effective/manual/embedded location provenance, primary original path/hash/size/extension, and derived-version IDs, state, provider, timestamps, hashes and managed-relative paths where available. It contains no binary pixels, prompts, recipes, or credentials. Output is streamed to a staged UTF-8 JSON file, syntax-validated, synchronised and then promoted, so a partial export is never presented as successful.

## Missing files, relinking and moved libraries

**Scan library** checks records without loading image pixels. It reports missing originals, missing derived versions, changed original size/hash, orphaned catalogue records, untracked files under managed `Originals`/`Edits`, and changed or missing sidecars that Keepframe exported. It only updates the catalogue's visible missing state; it does not delete anything.

For a missing selected original, **Relink selected** searches a folder you choose. Candidates must match the recorded size and SHA-256 exactly. More than one exact match requires an explicit user choice; no uncertain candidate is linked. The asset ID, tags, decisions and audit history are retained.

When a managed library has been copied or moved (including a Windows drive-letter change), opening it rebases only `Originals`, `Edits` and thumbnail paths that exist at the new root and verify against their stored hash where one exists. No arbitrary disk search is performed. External/original paths that cannot be verified remain missing and use the relink workflow.

## Folder-watch Inbox

Folder watches are optional and only apply to folders selected in Settings. The native watcher recursively listens to those folders, debounces repeated filesystem notifications for 600 ms, and deduplicates findings into the local **Changes-detected Inbox** as new, changed or removed. Events never import files, delete catalogue records, alter images, or relink an asset. Disabling watches stops monitoring while retaining Inbox findings for review.

## Privacy and limitations

Everything in this guide runs locally. Exports can contain filenames, paths, hashes, tags and GPS, so share them deliberately. Keepframe is not a Lightroom catalogue importer or Adobe database reader. Sidecar import is a safe subset rather than a complete XMP editor; it does not merge captions, ratings, face data, proprietary adjustments, unknown namespaces or raw-development settings.

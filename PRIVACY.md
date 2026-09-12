# Keepframe beta privacy notice

Keepframe is local-first. Catalogue operations, triage, tags, GPS placement, deterministic recipes and local preview generation stay on the user’s computer.

## Network use

- **Map:** the Map view requests only configured tile-provider tiles needed for the visible area. The tile host receives normal network information such as IP address and requested tile coordinates. Catalogue data and photographs are not sent with tile requests. The development default is OpenStreetMap; production deployments must configure a provider appropriate to their usage.
- **Place search:** disabled by default. When explicitly configured, Keepframe sends only the typed place-query text to that provider, debounces requests and keeps a short in-memory result cache. It does not send catalogue GPS, filenames, thumbnails or photographs. Automatic and bulk reverse geocoding are not enabled.
- **Local AI:** Keepframe permits only loopback HTTP services (`127.0.0.1`, `localhost` or `::1`). Images are not sent to a non-loopback AI endpoint.
- **Intelligent masking:** Subject, combined People and Sky predictions use an optional model and authenticated loopback worker on this computer. Analysis-size image pixels are held transiently for inference and are not uploaded. Only a compressed accepted coverage mask and creation provenance enter the Develop recipe; model weights remain outside the catalogue under `D:\AI Models\Keepframe`.
- **ChatGPT and Gemini:** Keepframe does not call their APIs. It prepares an sRGB PNG and provider-specific prompt locally for the user to transfer manually. The chosen external provider’s terms apply after the user uploads those files.
- **Updates and telemetry:** this beta contains no updater, analytics, crash uploader or background telemetry.

Diagnostics are exported only on explicit request. They contain application/schema versions, catalogue integrity and aggregate counts; they exclude image pixels, prompts, recipes and model inputs.

## Portability exports and folder watching

- **XMP sidecars and portable catalogue JSON:** created only after an explicit local action. They can contain filenames, managed-relative or original paths, hashes, tags, triage state, Develop recipes (including compressed accepted semantic-mask coverage and model provenance), derived-version provenance and GPS coordinates. Keepframe does not upload these exports, but anyone you share them with can read that metadata.
- **XMP import:** reads a conservative local subset and does not upload a sidecar. Unknown XMP is left untouched; existing differing catalogue values are reported as conflicts rather than overwritten.
- **Folder watching:** disabled unless a user chooses a folder in Settings. Native filesystem notifications stay local and are recorded only as a Changes-detected Inbox. Keepframe does not automatically import, delete, hash every file continuously, or transmit watcher findings.

The selected master library and optional models are external user data. Uninstalling Keepframe must not remove either.

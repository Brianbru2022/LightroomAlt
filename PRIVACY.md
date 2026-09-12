# Keepframe beta privacy notice

Keepframe is local-first. Catalogue operations, triage, tags, GPS placement, deterministic recipes and local preview generation stay on the user’s computer.

## Network use

- **Map:** the Map view requests only configured tile-provider tiles needed for the visible area. The tile host receives normal network information such as IP address and requested tile coordinates. Catalogue data and photographs are not sent with tile requests. The development default is OpenStreetMap; production deployments must configure a provider appropriate to their usage.
- **Place search:** disabled by default. When explicitly configured, Keepframe sends only the typed place-query text to that provider, debounces requests and keeps a short in-memory result cache. It does not send catalogue GPS, filenames, thumbnails or photographs. Automatic and bulk reverse geocoding are not enabled.
- **Local AI:** Keepframe permits only loopback HTTP services (`127.0.0.1`, `localhost` or `::1`). Images are not sent to a non-loopback AI endpoint.
- **ChatGPT and Gemini:** Keepframe does not call their APIs. It prepares an sRGB PNG and provider-specific prompt locally for the user to transfer manually. The chosen external provider’s terms apply after the user uploads those files.
- **Updates and telemetry:** this beta contains no updater, analytics, crash uploader or background telemetry.

Diagnostics are exported only on explicit request. They contain application/schema versions, catalogue integrity and aggregate counts; they exclude image pixels, prompts, recipes and model inputs.

The selected master library and optional models are external user data. Uninstalling Keepframe must not remove either.

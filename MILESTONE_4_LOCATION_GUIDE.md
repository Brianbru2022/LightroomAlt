# Location and map behaviour

Keepframe stores effective map coordinates in its local SQLite catalogue. Imported EXIF GPS is retained as **embedded GPS**. A point placed or moved in Keepframe is stored as a separate **manual pin**; clearing that pin restores the embedded coordinate when one exists. Keepframe does not rewrite source EXIF metadata.

## Map providers

The development default uses OpenStreetMap standard tiles with required attribution. This is a development convenience, not a production CDN commitment. Configure a production tile provider at build time without committing credentials:

```text
VITE_KEEPFRAME_TILE_URL=https://tiles.example/{z}/{x}/{y}.png
VITE_KEEPFRAME_TILE_ATTRIBUTION=Your required attribution HTML
VITE_KEEPFRAME_TILE_MAX_ZOOM=19
```

The URL must be an HTTPS tile template containing `{z}`, `{x}` and `{y}` (loopback HTTP is accepted for local development). Invalid or missing production configuration falls back to the attributed development provider. If tiles fail, marker and catalogue interactions remain available.

## Optional place search

No geocoder is enabled by default. To enable one, select a provider whose browser use, CORS policy and terms permit the integration, then configure:

```text
VITE_KEEPFRAME_GEOCODER_URL=https://geocoder.example/search?q={query}&app={app}
VITE_KEEPFRAME_GEOCODER_APPLICATION=keepframe-beta
```

The request is debounced, recent results are cached for the session, and only the typed place-query text leaves the machine. Keepframe never sends catalogue records, filenames, thumbnails or GPS coordinates for place search. If the provider is absent or fails, place search shows a clear local error and the map continues to work.

Keepframe does not perform automatic or bulk reverse geocoding. Raw GPS remains local and is still useful for map browsing, clustering and the explicit visible-map-area filter. Named country, region and town groups are intentionally not inferred from coordinates without a configured, user-requested reverse-geocoding operation.

## Offline and privacy behaviour

Map tiles are network requests to the configured tile service and reveal requested tile coordinates plus normal network metadata such as IP address. A map/tile/search outage never blocks catalogue opening, browsing, triage or editing. Offline maps are not implemented.

import L from "leaflet";
import "leaflet.markercluster";
import "leaflet.markercluster/dist/MarkerCluster.css";
import "leaflet.markercluster/dist/MarkerCluster.Default.css";
import { LocateFixed, MapPin, MousePointer2, Navigation, Search, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { geocoderAvailable, searchPlaces, tileProvider } from "../lib/mapServices";
import type { Asset, MapAsset, MapBounds } from "../types";

type Props = {
  assets: Asset[]; mapAssets: MapAsset[]; mapLoading: boolean; total: number; selected: Asset | null;
  onSelect: (id: string) => void; onLocations: (ids: string[], latitude: number, longitude: number) => void;
  onClearManualLocation: (id: string) => void; onOpenSelected: () => void; onLocatedFilter: (located: boolean | undefined) => void;
  spatialBounds?: MapBounds; onUseVisibleBounds: (bounds: MapBounds) => void; onClearSpatialBounds: () => void;
};

const MAP_LIST_ROW_HEIGHT = 52;
const locationLabel = (asset: { locationSource?: string; latitude?: number; longitude?: number }) => asset.latitude == null || asset.longitude == null
  ? "Location not set" : `${asset.latitude.toFixed(4)}, ${asset.longitude.toFixed(4)} · ${asset.locationSource === "manual" ? "manual pin" : "embedded GPS"}`;

export function MapView({ assets, mapAssets, mapLoading, total, selected, onSelect, onLocations, onClearManualLocation, onOpenSelected, onLocatedFilter, spatialBounds, onUseVisibleBounds, onClearSpatialBounds }: Props) {
  const mapNode = useRef<HTMLDivElement>(null);
  const mapRef = useRef<L.Map | null>(null);
  const clusterRef = useRef<L.MarkerClusterGroup | null>(null);
  const fittedRef = useRef(false);
  const [tilesUnavailable, setTilesUnavailable] = useState(false);
  const [mapReady, setMapReady] = useState(false);
  const [listScrollTop, setListScrollTop] = useState(0);
  const [moving, setMoving] = useState(false);
  const [placementIds, setPlacementIds] = useState<string[]>([]);
  const [placeQuery, setPlaceQuery] = useState("");
  const [placeResults, setPlaceResults] = useState<Array<{ label: string; latitude: number; longitude: number }>>([]);
  const [placeError, setPlaceError] = useState<string | null>(null);
  const unlocated = useMemo(() => assets.filter((asset) => asset.latitude == null || asset.longitude == null), [assets]);
  const listStart = Math.max(0, Math.floor(listScrollTop / MAP_LIST_ROW_HEIGHT) - 3);
  const visibleUnlocated = unlocated.slice(listStart, listStart + 14);
  const selectedForPlacement = selected && (selected.latitude == null || selected.longitude == null) ? [selected.id] : [];
  const placement = [...new Set([...placementIds, ...selectedForPlacement])];
  const selectRef = useRef(onSelect); selectRef.current = onSelect;
  const locationsRef = useRef(onLocations); locationsRef.current = onLocations;
  const provider = tileProvider();
  const searchEnabled = geocoderAvailable();

  useEffect(() => {
    if (!mapNode.current || mapRef.current) return;
    try {
      const map = L.map(mapNode.current, { zoomControl: true, attributionControl: true }).setView([56.24, -2.85], 8);
      const tiles = L.tileLayer(provider.url, { maxZoom: provider.maxZoom, attribution: provider.attribution });
      tiles.on("load", () => { setMapReady(true); setTilesUnavailable(false); });
      tiles.on("tileerror", () => { setTilesUnavailable(true); setMapReady(true); });
      tiles.addTo(map);
      const clusters = L.markerClusterGroup({ showCoverageOnHover: false, spiderfyOnMaxZoom: true, zoomToBoundsOnClick: true, maxClusterRadius: 56 });
      clusters.addTo(map); clusterRef.current = clusters; mapRef.current = map;
      const observer = new ResizeObserver(() => map.invalidateSize({ animate: false })); observer.observe(mapNode.current);
      const timeout = window.setTimeout(() => setMapReady(true), 8000); window.setTimeout(() => map.invalidateSize({ animate: false }), 0);
      return () => { window.clearTimeout(timeout); observer.disconnect(); map.remove(); mapRef.current = null; clusterRef.current = null; };
    } catch { setTilesUnavailable(true); setMapReady(true); }
  }, [provider.attribution, provider.maxZoom, provider.url]);

  useEffect(() => {
    const map = mapRef.current; const clusters = clusterRef.current; if (!map || !clusters) return;
    clusters.clearLayers();
    clusters.addLayers(mapAssets.map((asset) => {
      const button = document.createElement("button"); button.className = "photo-marker"; button.title = `${asset.filename} (${asset.locationSource === "manual" ? "manual pin" : "embedded GPS"})`;
      const image = document.createElement("img"); image.src = asset.thumbnailUrl; image.alt = ""; button.appendChild(image);
      button.addEventListener("click", () => selectRef.current(asset.id));
      const icon = L.divIcon({ className: "photo-marker-shell", html: button, iconSize: [46, 46], iconAnchor: [23, 46] });
      return L.marker([asset.latitude, asset.longitude], { icon, title: asset.filename });
    }));
    if (!fittedRef.current && mapAssets.length > 1) { map.fitBounds(L.latLngBounds(mapAssets.map((asset) => [asset.latitude, asset.longitude])), { padding: [70, 70], maxZoom: 12, animate: false }); fittedRef.current = true; }
  }, [mapAssets]);

  useEffect(() => { if (selected?.latitude != null && selected.longitude != null) mapRef.current?.setView([selected.latitude, selected.longitude], Math.max(mapRef.current.getZoom(), 10), { animate: true }); }, [selected?.id, selected?.latitude, selected?.longitude]);
  useEffect(() => {
    const map = mapRef.current; if (!map || (!moving && !placement.length)) return;
    const placeAtClick = (event: L.LeafletMouseEvent) => { const ids = moving && selected ? [selected.id] : placement; if (!ids.length) return; locationsRef.current(ids, event.latlng.lat, event.latlng.lng); setMoving(false); setPlacementIds([]); };
    map.on("click", placeAtClick); return () => { map.off("click", placeAtClick); };
  }, [moving, placement.join(","), selected?.id]);
  useEffect(() => {
    if (!placeQuery.trim()) { setPlaceResults([]); setPlaceError(null); return; }
    const controller = new AbortController(); const timer = window.setTimeout(() => { searchPlaces(placeQuery, controller.signal).then(setPlaceResults).catch((error: unknown) => { if (!controller.signal.aborted) { setPlaceResults([]); setPlaceError(error instanceof Error ? error.message : "Location search is unavailable."); } }); }, 350);
    return () => { controller.abort(); window.clearTimeout(timer); };
  }, [placeQuery]);
  const placeAtCentre = () => { if (!placement.length || !mapRef.current) return; const centre = mapRef.current.getCenter(); onLocations(placement, centre.lat, centre.lng); setPlacementIds([]); };
  const togglePlacement = (id: string) => setPlacementIds((current) => current.includes(id) ? current.filter((item) => item !== id) : [...current, id]);

  return <main className="view map-view">
    <div ref={mapNode} className={`map-canvas ${tilesUnavailable ? "map-fallback" : ""} ${(moving || placement.length) ? "placement-mode" : ""}`}>
      {tilesUnavailable ? <div className="map-fallback-copy"><Navigation size={24} /><strong>Base map unavailable</strong><span>Photo positions and catalogue browsing remain usable.</span></div> : null}
      {!mapReady ? <div className="map-loading">Loading map…</div> : null}
      {(moving || placement.length) ? <div className="placement-hint"><MapPin size={16} /><span>Click the map to {moving ? `move ${selected?.filename ?? "this photograph"}` : `place ${placement.length} selected photograph${placement.length === 1 ? "" : "s"}`}</span></div> : null}
    </div>
    <aside className="map-panel">
      <span className="eyebrow">Photo atlas</span><h1>{mapAssets.length.toLocaleString()} located photographs</h1><p>{spatialBounds ? "Showing the temporary visible-map area." : "All geotagged photographs matching the active catalogue filters."} {mapLoading ? "Refreshing markers…" : ""}</p>
      <label className="map-search"><Search size={15} /><input aria-label="Search for a place" value={placeQuery} disabled={!searchEnabled} placeholder={searchEnabled ? "Search a place" : "Location search not configured"} onChange={(event) => setPlaceQuery(event.target.value)} /></label>
      {placeResults.length ? <div className="place-results">{placeResults.map((result) => <button key={`${result.label}-${result.latitude}-${result.longitude}`} onClick={() => { mapRef.current?.setView([result.latitude, result.longitude], 12, { animate: true }); setPlaceResults([]); }}><strong>{result.label}</strong><small>{result.latitude.toFixed(4)}, {result.longitude.toFixed(4)}</small></button>)}</div> : null}
      {placeError ? <p className="map-service-note">{placeError}</p> : null}
      <div className="map-filter-row"><button className={spatialBounds ? "quiet-button active" : "quiet-button"} onClick={() => mapRef.current && onUseVisibleBounds({ south: mapRef.current.getBounds().getSouth(), west: mapRef.current.getBounds().getWest(), north: mapRef.current.getBounds().getNorth(), east: mapRef.current.getBounds().getEast() })}>Use visible area</button>{spatialBounds ? <button className="icon-button" aria-label="Clear visible-area filter" onClick={onClearSpatialBounds}><X size={15} /></button> : null}</div>
      <div className="map-filter-row"><button className="quiet-button" onClick={() => onLocatedFilter(true)}>Located</button><button className="quiet-button" onClick={() => onLocatedFilter(false)}>Unlocated</button><button className="quiet-button" onClick={() => onLocatedFilter(undefined)}>All</button></div>
      {selected ? <div className="map-selection"><img src={selected.thumbnailUrl} alt="" /><div><strong>{selected.filename}</strong><span>{locationLabel(selected)}</span><button className="text-button" onClick={onOpenSelected}>Open photograph</button></div></div> : null}
      {selected?.latitude != null && selected.longitude != null ? <div className="map-actions"><button className="quiet-button full" onClick={() => setMoving(true)}><MapPin size={15} /> Move selected point</button>{selected.locationSource === "manual" ? <button className="quiet-button full" onClick={() => onClearManualLocation(selected.id)}>Clear manual pin</button> : null}</div> : null}
      {placement.length ? <button className="primary-button full" onClick={placeAtCentre}><MapPin size={16} /> Place {placement.length} at map centre</button> : null}
      <div className="map-list-title"><span>Unlocated in current library page</span><strong>{unlocated.length}</strong></div>
      <div className="unlocated-list" onScroll={(event) => setListScrollTop(event.currentTarget.scrollTop)}><div className="virtual-map-list" style={{ height: unlocated.length * MAP_LIST_ROW_HEIGHT }}><div style={{ transform: `translateY(${listStart * MAP_LIST_ROW_HEIGHT}px)` }}>{visibleUnlocated.map((asset) => <label key={asset.id} className={selected?.id === asset.id ? "active" : ""}><input aria-label={`Place ${asset.filename}`} type="checkbox" checked={placementIds.includes(asset.id)} onChange={() => togglePlacement(asset.id)} /><button onClick={() => onSelect(asset.id)}><img src={asset.thumbnailUrl} alt="" /><span>{asset.filename}<small><MousePointer2 size={11} /> Select or include in placement</small></span></button></label>)}</div></div></div>
      <div className="map-privacy"><LocateFixed size={15} /><span>GPS remains local. Map tiles use {provider.label}; location search is optional and sends only its typed query.</span></div><small className="map-total">{total.toLocaleString()} photographs match the current catalogue filter.</small>
    </aside>
  </main>;
}

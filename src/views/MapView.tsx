import L from "leaflet";
import { LocateFixed, MapPin, MousePointer2, Navigation } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Asset } from "../types";

type Props = {
  assets: Asset[];
  total: number;
  hasMore: boolean;
  loading: boolean;
  onLoadMore: () => void;
  selected: Asset | null;
  onSelect: (asset: Asset) => void;
  onLocation: (id: string, latitude: number, longitude: number) => void;
};

const MAP_LIST_ROW_HEIGHT = 48;

export function MapView({ assets, total, hasMore, loading, onLoadMore, selected, onSelect, onLocation }: Props) {
  const mapNode = useRef<HTMLDivElement>(null);
  const mapRef = useRef<L.Map | null>(null);
  const markersRef = useRef<L.Marker[]>([]);
  const [tilesUnavailable, setTilesUnavailable] = useState(false);
  const [mapReady, setMapReady] = useState(false);
  const [listScrollTop, setListScrollTop] = useState(0);
  const unlocated = useMemo(() => assets.filter((asset) => asset.latitude === undefined || asset.longitude === undefined), [assets]);
  const located = useMemo(() => assets.filter((asset) => asset.latitude !== undefined && asset.longitude !== undefined), [assets]);
  const listStart = Math.max(0, Math.floor(listScrollTop / MAP_LIST_ROW_HEIGHT) - 3);
  const visibleUnlocated = unlocated.slice(listStart, listStart + 14);
  const selectRef = useRef(onSelect); selectRef.current = onSelect;
  const locationRef = useRef(onLocation); locationRef.current = onLocation;

  useEffect(() => {
    if (!mapNode.current || mapRef.current) return;
    try {
      const map = L.map(mapNode.current, { zoomControl: true, attributionControl: true }).setView([56.24, -2.85], 8);
      const tiles = L.tileLayer("https://tile.openstreetmap.org/{z}/{x}/{y}.png", { maxZoom: 19, attribution: "&copy; OpenStreetMap contributors" });
      tiles.on("load", () => { setMapReady(true); setTilesUnavailable(false); });
      tiles.on("tileerror", () => { setTilesUnavailable(true); setMapReady(true); });
      tiles.addTo(map);
      mapRef.current = map;
      const observer = new ResizeObserver(() => map.invalidateSize({ animate: false }));
      observer.observe(mapNode.current);
      const timeout = window.setTimeout(() => setMapReady(true), 8000);
      window.setTimeout(() => map.invalidateSize({ animate: false }), 0);
      return () => { window.clearTimeout(timeout); observer.disconnect(); map.remove(); mapRef.current = null; };
    } catch {
      setTilesUnavailable(true);
      setMapReady(true);
    }
  }, []);

  useEffect(() => {
    const map = mapRef.current; if (!map) return;
    markersRef.current.forEach((marker) => marker.remove());
    markersRef.current = located.map((asset) => {
      const button = document.createElement("button");
      button.className = `photo-marker ${selected?.id === asset.id ? "active" : ""}`;
      button.title = asset.filename;
      const image = document.createElement("img");
      image.src = asset.thumbnailUrl;
      image.alt = "";
      button.appendChild(image);
      button.addEventListener("click", () => selectRef.current(asset));
      const size = selected?.id === asset.id ? 58 : 48;
      const icon = L.divIcon({ className: "photo-marker-shell", html: button, iconSize: [size, size], iconAnchor: [size / 2, size] });
      return L.marker([asset.latitude!, asset.longitude!], { icon, title: asset.filename }).addTo(map);
    });
    if (located.length > 1 && selected?.latitude === undefined) {
      map.fitBounds(L.latLngBounds(located.map((asset) => [asset.latitude!, asset.longitude!])), { padding: [70, 70], maxZoom: 12, animate: false });
    }
  }, [located, selected?.id, selected?.latitude]);

  useEffect(() => {
    if (selected?.latitude !== undefined && selected.longitude !== undefined) {
      mapRef.current?.setView([selected.latitude, selected.longitude], Math.max(mapRef.current.getZoom(), 10), { animate: true });
    }
  }, [selected?.id, selected?.latitude, selected?.longitude]);

  useEffect(() => {
    const map = mapRef.current;
    if (!map || !selected || selected.latitude !== undefined) return;
    const placeAtClick = (event: L.LeafletMouseEvent) => {
      locationRef.current(selected.id, event.latlng.lat, event.latlng.lng);
    };
    map.on("click", placeAtClick);
    return () => { map.off("click", placeAtClick); };
  }, [selected?.id, selected?.latitude]);

  const placeSelected = () => {
    if (!selected || selected.latitude !== undefined || !mapRef.current) return;
    const centre = mapRef.current.getCenter();
    locationRef.current(selected.id, centre.lat, centre.lng);
  };

  return (
    <main className="view map-view">
      <div ref={mapNode} className={`map-canvas ${tilesUnavailable ? "map-fallback" : ""} ${selected?.latitude === undefined ? "placement-mode" : ""}`}>
        {tilesUnavailable ? <div className="map-fallback-copy"><Navigation size={24} /><strong>Base map unavailable</strong><span>Photo positions remain usable.</span></div> : null}
        {!mapReady ? <div className="map-loading">Loading map…</div> : null}
        {selected && selected.latitude === undefined ? <div className="placement-hint"><MapPin size={16} /><span>Click the map to place {selected.filename}</span></div> : null}
      </div>
      <aside className="map-panel">
        <span className="eyebrow">Photo atlas</span><h1>{located.length} located photographs</h1><p>{assets.length} of {total} filtered photographs loaded. Browse photographs already carrying GPS coordinates.</p>
        {selected ? <div className="map-selection"><img src={selected.thumbnailUrl} alt="" /><div><strong>{selected.filename}</strong><span>{selected.latitude === undefined ? "Location not set" : `${selected.latitude.toFixed(4)}, ${selected.longitude?.toFixed(4)}`}</span></div></div> : null}
        {selected?.latitude === undefined ? <button className="primary-button full" onClick={placeSelected}><MapPin size={16} /> Place at current map centre</button> : null}
        <div className="map-list-title"><span>Without a location</span><strong>{unlocated.length}</strong></div>
        <div className="unlocated-list" onScroll={(event) => {
          const node = event.currentTarget;
          setListScrollTop(node.scrollTop);
          if (hasMore && !loading && node.scrollTop + node.clientHeight >= node.scrollHeight - 200) onLoadMore();
        }}>
          <div className="virtual-map-list" style={{ height: unlocated.length * MAP_LIST_ROW_HEIGHT }}>
            <div style={{ transform: `translateY(${listStart * MAP_LIST_ROW_HEIGHT}px)` }}>
              {visibleUnlocated.map((asset) => <button key={asset.id} className={selected?.id === asset.id ? "active" : ""} onClick={() => onSelect(asset)}><img src={asset.thumbnailUrl} alt="" /><span>{asset.filename}<small><MousePointer2 size={11} /> Select, then click map</small></span></button>)}
            </div>
          </div>
        </div>
        {hasMore ? <button className="quiet-button full" disabled={loading} onClick={onLoadMore}>{loading ? "Loading…" : `Load more (${assets.length} of ${total})`}</button> : null}
        <div className="map-privacy"><LocateFixed size={15} /><span>GPS stays in your local catalogue. Only visible map tiles are requested from OpenStreetMap.</span></div>
      </aside>
    </main>
  );
}

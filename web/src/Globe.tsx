import { useEffect, useRef, useState } from "react";
import {
  Cartesian3, Color, EllipsoidTerrainProvider,
  ImageryLayer, Math as CesiumMath, OpenStreetMapImageryProvider, Viewer
} from "cesium";
import ms from "milsymbol";
import "cesium/Build/Cesium/Widgets/widgets.css";
import { AirportLayer, type AirportDetail, type AirportListResponse } from "./airportLayer";
import { GlobeEntityReconciler, type Projection } from "./globeEntities";
import { attachMapKeyboardControls, type AttachedMapKeyboardControls } from "./mapKeyboardControls";
import type { MapFilters } from "./MapFilterDialog";
import { SpaceAssetLayer } from "./spaceAssetLayer";
import { apiRequest } from "./apiClient";

const API_BASE = import.meta.env.VITE_API_BASE ?? "";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  return apiRequest<T>(API_BASE, path, init);
}

const symbolCache = new Map<string, HTMLCanvasElement>();
function symbolCanvas(sidc: string, size = 32) {
  const key = `${sidc}:${size}`;
  let canvas = symbolCache.get(key);
  if (!canvas) {
    canvas = new ms.Symbol(sidc, { size, frame: true, fill: true }).asCanvas();
    symbolCache.set(key, canvas);
  }
  return canvas;
}

export default function Globe({ projection, filters, gameId, playerId, roleId, spaceCatalogEnabled, keyboardEnabled, renderingEnabled }: {
  projection: Projection;
  filters: MapFilters;
  gameId: string;
  playerId: string;
  roleId: string;
  spaceCatalogEnabled: boolean;
  keyboardEnabled: boolean;
  renderingEnabled: boolean;
}) {
  const host = useRef<HTMLDivElement>(null);
  const viewerRef = useRef<Viewer | null>(null);
  const reconcilerRef = useRef<GlobeEntityReconciler | null>(null);
  const airportLayerRef = useRef<AirportLayer | null>(null);
  const spaceAssetLayerRef = useRef<SpaceAssetLayer | null>(null);
  const keyboardControlsRef = useRef<AttachedMapKeyboardControls | null>(null);
  const showSpaceAssetsRef = useRef<() => void>(() => undefined);
  const airportRequestRef = useRef<AbortController | undefined>(undefined);
  const refreshAirportsRef = useRef<() => void>(() => undefined);
  const filtersRef = useRef(filters);
  const focusedRef = useRef(false);
  filtersRef.current = filters;
  const [airportStatus, setAirportStatus] = useState("Loading airports");
  const [spaceAssetStatus, setSpaceAssetStatus] = useState("Space assets hidden");

  useEffect(() => {
    if (!host.current || viewerRef.current) return;
    const viewer = new Viewer(host.current, {
      animation: false,
      baseLayer: new ImageryLayer(new OpenStreetMapImageryProvider({ url: "https://tile.openstreetmap.org/", credit: "OpenStreetMap contributors" })),
      baseLayerPicker: false, fullscreenButton: false, geocoder: false, homeButton: false,
      infoBox: true, navigationHelpButton: false, sceneModePicker: false, selectionIndicator: true,
      terrainProvider: new EllipsoidTerrainProvider(), timeline: false,
      requestRenderMode: true, maximumRenderTimeChange: Infinity, targetFrameRate: 30
    });
    viewer.scene.globe.baseColor = Color.fromCssColorString("#1f3340");
    viewer.camera.setView({ destination: Cartesian3.fromDegrees(-40, 30, 20_000_000) });
    viewerRef.current = viewer;
    const keyboardControls = attachMapKeyboardControls(viewer.camera, viewer.scene.globe.ellipsoid);
    keyboardControls.setEnabled(keyboardEnabled);
    keyboardControlsRef.current = keyboardControls;
    reconcilerRef.current = new GlobeEntityReconciler(viewer.entities, symbolCanvas);
    const airportLayer = new AirportLayer(viewer, (airportId) =>
      request<AirportDetail>(`/v1/airports/${encodeURIComponent(airportId)}`)
    );
    airportLayerRef.current = airportLayer;
    const showSpaceAssets = () => {
      if (!spaceAssetLayerRef.current) {
        spaceAssetLayerRef.current = new SpaceAssetLayer(viewer, gameId, playerId, roleId, setSpaceAssetStatus);
      }
      spaceAssetLayerRef.current.setFilters(filtersRef.current.spaceAssets);
    };
    showSpaceAssetsRef.current = showSpaceAssets;
    if (spaceCatalogEnabled && (filtersRef.current.spaceAssets.showAll || filtersRef.current.spaceAssets.showStarlink)) showSpaceAssets();
    let refreshTimer: number | undefined;
    let stopped = false;

    const refreshAirports = async () => {
      const runwayFilters = filtersRef.current.runways;
      if (!runwayFilters.visible) return;
      const rectangle = viewer.camera.computeViewRectangle(viewer.scene.globe.ellipsoid);
      if (!rectangle) return;
      airportRequestRef.current?.abort();
      airportRequestRef.current = new AbortController();
      const cameraPosition = viewer.camera.positionCartographic;
      const ellipsoidRadius = viewer.scene.globe.ellipsoid.maximumRadius;
      const horizonRadius = Math.acos(Math.min(1, ellipsoidRadius / Math.max(ellipsoidRadius, ellipsoidRadius + cameraPosition.height)));
      const query = new URLSearchParams({
        west: CesiumMath.toDegrees(rectangle.west).toFixed(5),
        south: CesiumMath.toDegrees(rectangle.south).toFixed(5),
        east: CesiumMath.toDegrees(rectangle.east).toFixed(5),
        north: CesiumMath.toDegrees(rectangle.north).toFixed(5),
        horizon_latitude: CesiumMath.toDegrees(cameraPosition.latitude).toFixed(5),
        horizon_longitude: CesiumMath.toDegrees(cameraPosition.longitude).toFixed(5),
        horizon_radius_deg: CesiumMath.toDegrees(horizonRadius).toFixed(5),
        minimum_runway_length_m: String(runwayFilters.minimumLengthM),
        limit: "500"
      });
      try {
        const response = await request<AirportListResponse>(`/v1/airports?${query}`, { signal: airportRequestRef.current.signal });
        if (stopped || !filtersRef.current.runways.visible) return;
        airportLayer.update(response.airports);
        const threshold = runwayFilters.minimumLengthM > 0 ? ` · runways ≥ ${runwayFilters.minimumLengthM.toLocaleString()} m` : "";
        setAirportStatus(response.total > response.airports.length
          ? `${response.airports.length.toLocaleString()} of ${response.total.toLocaleString()} airports${threshold}`
          : `${response.total.toLocaleString()} airports${threshold}`);
      } catch (error) {
        if (!stopped && !(error instanceof DOMException && error.name === "AbortError")) {
          setAirportStatus("Airport layer unavailable");
        }
      }
    };
    refreshAirportsRef.current = () => void refreshAirports();
    const scheduleAirportRefresh = () => {
      if (refreshTimer !== undefined) window.clearTimeout(refreshTimer);
      refreshTimer = window.setTimeout(() => void refreshAirports(), 150);
    };
    viewer.camera.moveEnd.addEventListener(scheduleAirportRefresh);
    void refreshAirports();
    return () => {
      stopped = true;
      airportRequestRef.current?.abort();
      if (refreshTimer !== undefined) window.clearTimeout(refreshTimer);
      viewer.camera.moveEnd.removeEventListener(scheduleAirportRefresh);
      keyboardControls.destroy();
      spaceAssetLayerRef.current?.destroy();
      airportLayer.destroy();
      airportLayerRef.current = null;
      spaceAssetLayerRef.current = null;
      showSpaceAssetsRef.current = () => undefined;
      refreshAirportsRef.current = () => undefined;
      keyboardControlsRef.current = null;
      reconcilerRef.current = null;
      viewer.destroy();
      viewerRef.current = null;
    };
  }, [spaceCatalogEnabled]);

  useEffect(() => {
    keyboardControlsRef.current?.setEnabled(keyboardEnabled);
  }, [keyboardEnabled]);

  useEffect(() => {
    const viewer = viewerRef.current;
    if (!viewer) return;
    viewer.useDefaultRenderLoop = renderingEnabled;
    if (renderingEnabled) viewer.scene.requestRender();
  }, [renderingEnabled]);

  useEffect(() => {
    const viewer = viewerRef.current;
    if (!renderingEnabled || !viewer) return;
    reconcilerRef.current?.reconcile({
      ...projection,
      communication_links: filters.network.visible ? projection.communication_links : []
    });
    if (!focusedRef.current && viewerRef.current && reconcilerRef.current) {
      const entities = reconcilerRef.current.focusEntities();
      if (entities.length) {
        focusedRef.current = true;
        void viewerRef.current.flyTo(entities, { duration: 0 });
      }
    }
    viewer.scene.requestRender();
  }, [projection, filters.network.visible, renderingEnabled]);

  useEffect(() => {
    if (spaceCatalogEnabled && (filters.spaceAssets.showAll || filters.spaceAssets.showStarlink)) {
      showSpaceAssetsRef.current();
    } else {
      spaceAssetLayerRef.current?.setFilters(filters.spaceAssets);
      setSpaceAssetStatus("Space assets hidden");
    }
  }, [filters.spaceAssets.showAll, filters.spaceAssets.showStarlink, spaceCatalogEnabled]);

  useEffect(() => {
    airportRequestRef.current?.abort();
    if (!filters.runways.visible) {
      airportLayerRef.current?.hide();
      setAirportStatus("Airport runways hidden");
      return;
    }
    setAirportStatus("Loading airports");
    refreshAirportsRef.current();
  }, [filters.runways.visible, filters.runways.minimumLengthM]);

  return <><div className="globe" ref={host} aria-label="Operational map. Use W A S D to move and Q or E to turn." aria-keyshortcuts="W A S D Q E" /><div className="map-layer-status"><span>{airportStatus}</span><span>{spaceCatalogEnabled ? spaceAssetStatus : "No orbital catalog in this scenario"}</span></div><div className="map-controls-hint">WASD MOVE · Q/E TURN</div></>;
}


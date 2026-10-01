import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import {
  Cartesian3, Color, EllipsoidTerrainProvider,
  ImageryLayer, Math as CesiumMath, OpenStreetMapImageryProvider, Viewer
} from "cesium";
import ms from "milsymbol";
import "cesium/Build/Cesium/Widgets/widgets.css";
import "./styles.css";
import type { AuthorityDefinition, AuthorityRequest, Role } from "./AuthorityWorkspace";
import { AirportLayer, type AirportDetail, type AirportListResponse } from "./airportLayer";
import { GlobeEntityReconciler, type Projection } from "./globeEntities";
import { attachMapKeyboardControls, type AttachedMapKeyboardControls } from "./mapKeyboardControls";
import { MapFilterDialog, type MapFilters } from "./MapFilterDialog";
import { OperationalInspector } from "./OperationalInspector";
import { SpaceAssetLayer } from "./spaceAssetLayer";
import { ApiError, apiRequest } from "./apiClient";
import { usePollingResource, type PollingStatus } from "./usePollingResource";
import { GameSessionControls, GameSessionNotice, type Game } from "./GameSessionControls";

const AuthorityWorkspace = lazy(() => import("./AuthorityWorkspace").then((module) => ({ default: module.AuthorityWorkspace })));
const NetworkWorkspace = lazy(() => import("./NetworkWorkspace").then((module) => ({ default: module.NetworkWorkspace })));

const API_BASE = import.meta.env.VITE_API_BASE ?? "";
const SAVED_PASSWORD_MASK = "••••••••••••";
type Scenario = { id: string; title: string; description: string; version: number; authored_entity_count: number; role_count: number; requires_space_catalog: boolean };
type SpaceStatus = { setup_auth_required: boolean; remembered_credentials: boolean; remembered_username?: string; configured: boolean; syncing: boolean; usable: boolean; stale: boolean; using_cached_fallback: boolean; synced_unix?: number; age_seconds?: number; next_sync_unix?: number; object_count: number; checksum?: string; error?: string };
type SpaceTrackFeedback = { kind: "success" | "warning" | "error"; title: string; detail: string };

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  return apiRequest<T>(API_BASE, path, init);
}

function formatCatalogAge(seconds: number) {
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} minute${minutes === 1 ? "" : "s"} ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"} ago`;
  const days = Math.floor(hours / 24);
  return `${days} day${days === 1 ? "" : "s"} ago`;
}

function formatRefreshWait(seconds: number) {
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = seconds % 60;
  return `${minutes}m ${remainingSeconds.toString().padStart(2, "0")}s`;
}

function CatalogDownloadTime({ syncedUnix }: { syncedUnix: number }) {
  const [nowUnix, setNowUnix] = useState(() => Math.floor(Date.now() / 1000));
  useEffect(() => {
    setNowUnix(Math.floor(Date.now() / 1000));
    const timer = window.setInterval(() => setNowUnix(Math.floor(Date.now() / 1000)), 60_000);
    return () => window.clearInterval(timer);
  }, [syncedUnix]);
  const downloadedAt = new Date(syncedUnix * 1000);
  const absoluteTime = new Intl.DateTimeFormat(undefined, {
    year: "numeric", month: "short", day: "numeric",
    hour: "numeric", minute: "2-digit", second: "2-digit", timeZoneName: "short"
  }).format(downloadedAt);
  return <div className="space-catalog-timestamp">
    <span>Last downloaded</span>
    <strong><time dateTime={downloadedAt.toISOString()}>{absoluteTime}</time></strong>
    <small>{formatCatalogAge(Math.max(0, nowUnix - syncedUnix))}</small>
  </div>;
}

function CatalogTabStatus({ status }: { status: SpaceStatus | null }) {
  const presentation = !status
    ? { state: "checking", icon: "…", tooltip: "Checking space catalog status." }
    : status.usable && !status.stale
      ? { state: "ready", icon: "✓", tooltip: `Space catalog ready with ${status.object_count.toLocaleString()} recent objects.` }
      : status.usable
        ? { state: "stale", icon: "!", tooltip: `Space catalog available with ${status.object_count.toLocaleString()} objects, but it is stale and should be refreshed.` }
        : { state: "missing", icon: "×", tooltip: "Space catalog unavailable. Open Space Configuration to fetch space data." };
  return <span className={`catalog-tab-status ${presentation.state}`} data-tooltip={presentation.tooltip} aria-hidden="true">{presentation.icon}</span>;
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

function Globe({ projection, filters, gameId, playerId, roleId, spaceCatalogEnabled, keyboardEnabled, renderingEnabled }: {
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

function App() {
  const [scenarios, setScenarios] = useState<Scenario[]>([]);
  const [selectedScenarioId, setSelectedScenarioId] = useState("");
  const [games, setGames] = useState<Game[]>([]);
  const [spaceStatus, setSpaceStatus] = useState<SpaceStatus | null>(null);
  const [game, setGame] = useState<Game | null>(null);
  const [roles, setRoles] = useState<Role[]>([]);
  const [role, setRole] = useState<Role | null>(null);
  const [pendingControl, setPendingControl] = useState<"running" | "paused" | null>(null);
  const [controlError, setControlError] = useState("");
  const controlRequest = useRef<AbortController | null>(null);
  const controlPending = useRef(false);
  const activeGameId = useRef<string | null>(null);
  activeGameId.current = game?.id ?? null;
  const [mode, setMode] = useState<"new" | "join" | "space">("new");
  const [message, setMessage] = useState("Loading scenarios");
  const [gameTitle, setGameTitle] = useState("Global Crisis");
  const [displayName, setDisplayName] = useState("Commander");
  const [adminToken, setAdminToken] = useState("");
  const [spaceUsername, setSpaceUsername] = useState("");
  const [spacePassword, setSpacePassword] = useState("");
  const [rememberCredentials, setRememberCredentials] = useState(true);
  const [spaceTrackSyncing, setSpaceTrackSyncing] = useState(false);
  const [spaceTrackFeedback, setSpaceTrackFeedback] = useState<SpaceTrackFeedback | null>(null);
  const [nowUnix, setNowUnix] = useState(() => Math.floor(Date.now() / 1000));
  const [showAuthority, setShowAuthority] = useState(false);
  const [showNetwork, setShowNetwork] = useState(false);
  const [showMapFilters, setShowMapFilters] = useState(false);
  const [mapFilters, setMapFilters] = useState<MapFilters>({
    spaceAssets: { showAll: false, showStarlink: false },
    runways: { visible: true, minimumLengthM: 0 },
    network: { visible: false }
  });
  const [authority, setAuthority] = useState<AuthorityDefinition | null>(null);
  const [authorityRequests, setAuthorityRequests] = useState<AuthorityRequest[]>([]);
  const restoreAttempted = useRef(false);
  const playerId = useMemo(() => localStorage.getItem("world-at-war-player") ?? crypto.randomUUID(), []);
  const playable = (game?.status === "running" || game?.status === "paused") && role !== null;
  const refreshWaitSeconds = Math.max(0, (spaceStatus?.next_sync_unix ?? 0) - nowUnix);
  const catalogRefreshBlocked = refreshWaitSeconds > 0;
  const selectedScenario = scenarios.find((scenario) => scenario.id === selectedScenarioId) ?? scenarios[0];
  const usingSavedCredentials = Boolean(spaceStatus?.remembered_credentials && spacePassword === SAVED_PASSWORD_MASK);
  const gameResource = usePollingResource<Game>(game ? game.id + ":" + playerId : null, async (signal) => {
    const loaded = await request<Game[]>("/v1/games", { signal });
    const current = loaded.find((item) => item.id === game!.id);
    if (!current) throw new ApiError("This game is no longer available. Return to the lobby to choose another scenario.", 404);
    return current;
  }, 2_000);
  const roleResource = usePollingResource<Role[]>(game ? game.id + ":" + (role?.id ?? "") + ":" + (role?.lease_generation ?? "") : null,
    (signal) => request<Role[]>("/v1/games/" + game!.id + "/roles", { signal }), 2_000);
  const authorityResource = usePollingResource<{ definition: AuthorityDefinition; requests: AuthorityRequest[] }>(
    game && (role || game.host_player_id === playerId) ? game.id + ":" + playerId + ":" + (role?.id ?? "") + ":" + (role?.lease_generation ?? "") : null,
    async (signal) => {
      const query = new URLSearchParams({ player_id: playerId });
      const [definition, requests] = await Promise.all([
        request<AuthorityDefinition>("/v1/games/" + game!.id + "/authority?" + query, { signal }),
        request<AuthorityRequest[]>("/v1/games/" + game!.id + "/authority/requests?" + query + (role ? "&role_id=" + role.id : ""), { signal })
      ]);
      return { definition, requests };
    }, 2_000);
  const projectionResource = usePollingResource<Projection>(
    playable && game && role ? game.id + ":" + playerId + ":" + role.id + ":" + role.lease_generation : null,
    (signal) => request<Projection>("/v1/games/" + game!.id + "/state?" + new URLSearchParams({ player_id: playerId, role_id: role!.id }), { signal }));
  const projection = projectionResource.data;
  const connectionStatus: PollingStatus = [projectionResource.status, gameResource.status].includes("unavailable") ? "unavailable"
    : [projectionResource.status, gameResource.status].includes("reconnecting") ? "reconnecting"
    : projectionResource.status === "live" && gameResource.status === "live" ? "live" : "connecting";
  const connectionError = projectionResource.error ?? gameResource.error;
  const canIssueOrders = game?.status === "running" && connectionStatus === "live" && !game.operational_error;
  const retryConnection = () => { projectionResource.refresh(); gameResource.refresh(); roleResource.refresh(); authorityResource.refresh(); };
  const authorityUnits = useMemo(() => {
    if (projection?.own_units.length) return projection.own_units;
    const ids = new Set<string>();
    for (const item of roles) { ids.add(item.location_unit_id); item.command_units.forEach((id) => ids.add(id)); }
    return Array.from(ids, (id) => ({ id, name: `Unit ${id.slice(-6)}`, domain: "Command" }));
  }, [projection, roles]);

  async function refreshLobby() {
    const [loadedScenarios, loadedGames, status] = await Promise.all([
      request<Scenario[]>("/v1/scenarios"), request<Game[]>("/v1/games"), request<SpaceStatus>("/v1/settings/space-catalog/status")
    ]);
    let effectiveStatus = status;
    if (status.remembered_credentials && !status.configured && !restoreAttempted.current) {
      restoreAttempted.current = true;
      effectiveStatus = await request<SpaceStatus>("/v1/settings/space-track/credentials", { method: "POST" });
    }
    setScenarios(loadedScenarios); setGames(loadedGames); setSpaceStatus(effectiveStatus);
    setSelectedScenarioId((current) => current && loadedScenarios.some((scenario) => scenario.id === current) ? current : loadedScenarios[0]?.id ?? "");
    setGame((current) => current
      ? loadedGames.find((candidate) => candidate.id === current.id) ?? current
      : null);
  }

  useEffect(() => {
    localStorage.setItem("world-at-war-player", playerId);
    void refreshLobby().then(() => setMessage("Create a scenario or join a running game")).catch((error: Error) => setMessage(error.message));
  }, [playerId]);

  useEffect(() => {
    const nextSyncUnix = spaceStatus?.next_sync_unix;
    const currentUnix = Math.floor(Date.now() / 1000);
    setNowUnix(currentUnix);
    if (!nextSyncUnix || nextSyncUnix <= currentUnix) return;
    const timer = window.setInterval(() => {
      const updatedUnix = Math.floor(Date.now() / 1000);
      setNowUnix(updatedUnix);
      if (updatedUnix >= nextSyncUnix) window.clearInterval(timer);
    }, 1_000);
    return () => window.clearInterval(timer);
  }, [spaceStatus?.next_sync_unix]);

  useEffect(() => {
    if (!spaceStatus?.remembered_credentials) return;
    setRememberCredentials(true);
    setSpaceUsername((current) => current || spaceStatus.remembered_username || "");
    setSpacePassword((current) => current || SAVED_PASSWORD_MASK);
  }, [spaceStatus?.remembered_credentials, spaceStatus?.remembered_username]);

  useEffect(() => {
    const loaded = gameResource.data;
    if (loaded) setGame((current) => current?.id === loaded.id ? loaded : current);
  }, [gameResource.data]);

  useEffect(() => {
    if (!roleResource.data) return;
    setRoles(roleResource.data);
    if (role) {
      const current = roleResource.data.find((candidate) => candidate.id === role.id);
      if (!current?.held || current.lease_generation !== role.lease_generation) {
        setRole(null); setAuthorityRequests([]); setShowAuthority(false); setShowNetwork(false); setShowMapFilters(false);
        setMessage("Your role lease changed. Choose an available role to continue.");
      } else {
        setRole(current);
      }
    }
  }, [roleResource.data]);

  useEffect(() => {
    if (!authorityResource.data) return;
    setAuthority((current) => current?.version === authorityResource.data!.definition.version ? current : authorityResource.data!.definition);
    setAuthorityRequests(authorityResource.data.requests);
  }, [authorityResource.data]);

  useEffect(() => {
    const error = projectionResource.error;
    if (error instanceof ApiError && (error.status === 403 || error.status === 404)) {
      setRole(null); setAuthorityRequests([]); setShowAuthority(false); setShowNetwork(false); setShowMapFilters(false);
      setMessage("Your role is no longer available. Choose an available role to continue.");
    }
  }, [projectionResource.error]);

  useEffect(() => {
    setPendingControl(null); setControlError("");
    return () => { controlRequest.current?.abort(); controlPending.current = false; };
  }, [game?.id]);

  async function connectSpaceTrack() {
    setSpaceTrackSyncing(true);
    setSpaceTrackFeedback(null);
    setMessage(usingSavedCredentials
      ? "Downloading the public GP catalog with saved credentials. This can take a minute."
      : "Authenticating and downloading the public GP catalog. This can take a minute.");
    try {
      const status = await request<SpaceStatus>(usingSavedCredentials ? "/v1/admin/space-catalog/sync" : "/v1/admin/space-track/connect", {
        method: "POST", headers: { authorization: `Bearer ${adminToken}` },
        ...(usingSavedCredentials ? {} : { body: JSON.stringify({ username: spaceUsername, password: spacePassword, remember: rememberCredentials }) })
      });
      setSpaceStatus(status);
      if (!usingSavedCredentials && rememberCredentials) {
        setSpacePassword(SAVED_PASSWORD_MASK);
      } else if (!usingSavedCredentials) {
        setSpacePassword("");
      }
      setSpaceTrackFeedback(status.using_cached_fallback
        ? {
            kind: "warning",
            title: "Refresh failed; cached catalog is still available",
            detail: `${status.object_count.toLocaleString()} cached public objects are ready to use.`
          }
        : {
            kind: "success",
            title: "Catalog download complete",
            detail: `${status.object_count.toLocaleString()} public objects are ready to use.`
          });
      setMessage(status.using_cached_fallback
        ? `Catalog refresh failed; using ${status.object_count.toLocaleString()} cached public objects.`
        : `Catalog ready: ${status.object_count.toLocaleString()} public objects.`);
    } catch (error) {
      const detail = (error as Error).message;
      setSpaceTrackFeedback({ kind: "error", title: "Catalog download failed", detail });
      setMessage(`Space-Track synchronization failed: ${detail}`);
      void request<SpaceStatus>("/v1/settings/space-catalog/status").then(setSpaceStatus).catch(() => undefined);
    } finally {
      setSpaceTrackSyncing(false);
    }
  }

  async function forgetSpaceTrack() {
    try {
      const status = await request<SpaceStatus>("/v1/settings/space-track/credentials", { method: "DELETE" });
      setSpaceStatus(status); setSpaceUsername(""); setSpacePassword("");
      setMessage("Saved Space-Track credentials removed. The downloaded catalog remains available until it expires.");
    } catch (error) { setMessage((error as Error).message); }
  }

  async function createGame() {
    const scenario = selectedScenario; if (!scenario) return;
    try {
      const created = await request<{ game: Game }>("/v1/games", { method: "POST", body: JSON.stringify({ scenario_id: scenario.id, title: gameTitle, host_player_id: playerId }) });
      setGame(created.game); setRoles(await request<Role[]>(`/v1/games/${created.game.id}/roles`)); setMessage("Claim a role, then start the scenario.");
    } catch (error) { setMessage((error as Error).message); }
  }

  async function selectGame(selected: Game) {
    try {
      await request(`/v1/games/${selected.id}/join`, { method: "POST", body: JSON.stringify({ display_name: displayName }) });
      setGame(selected); setRole(null); setRoles(await request<Role[]>(`/v1/games/${selected.id}/roles`)); setMessage("Choose an available role.");
    } catch (error) { setMessage((error as Error).message); }
  }

  async function claim(selected: Role) {
    if (!game) return;
    try {
      const claimed = await request<Role>(`/v1/games/${game.id}/roles/${selected.id}/claim`, { method: "POST", body: JSON.stringify({ player_id: playerId }) });
      if (activeGameId.current !== game.id) return;
      setRole(claimed); setRoles((items) => items.map((item) => item.id === claimed.id ? claimed : item));
      setAuthorityRequests([]); setMessage(`${claimed.name} claimed.`);
    } catch (error) { setMessage((error as Error).message); }
  }

  async function controlGame(status: "running" | "paused") {
    if (!game || game.host_player_id !== playerId || controlPending.current) return;
    const controller = new AbortController();
    controlRequest.current = controller; controlPending.current = true;
    setPendingControl(status); setControlError("");
    try {
      const updated = await request<Game>("/v1/games/" + game.id + (status === "running" ? "/start" : "/pause"), {
        method: "POST", body: JSON.stringify({ player_id: playerId }), signal: controller.signal
      });
      if (controller.signal.aborted || activeGameId.current !== game.id) return;
      setGame(updated); gameResource.refresh();
      setMessage(status === "paused" ? "Scenario paused." : "Scenario running.");
    } catch (cause) {
      if (controller.signal.aborted || activeGameId.current !== game.id) return;
      const detail = cause instanceof Error ? cause.message : String(cause);
      setControlError(detail); setMessage(detail);
    } finally {
      if (controlRequest.current === controller) { controlPending.current = false; setPendingControl(null); }
    }
  }

  async function start() { await controlGame("running"); }

  async function turnNorth() {
    if (!game || !role || !projection || !canIssueOrders) return;
    const target = role.command_units[0]; if (!target) return;
    await request<{ status: "queued" | "pending_authority" }>(`/v1/games/${game.id}/roles/${role.id}/intent`, { method: "POST", body: JSON.stringify({ player_id: playerId, lease_generation: role.lease_generation, intent: { intent_id: crypto.randomUUID(), issuer_role: role.id, target, kind: { Move: { north_mps: 130, east_mps: 0 } }, requested_tick: projection.tick + 1 } }) }).then((outcome) => setMessage(outcome.status === "pending_authority" ? "Order awaiting authority approval." : "Order queued for delivery.")).catch((error: Error) => setMessage(error.message));
  }

  async function saveAuthority(draft: AuthorityDefinition) {
    if (!game || !authority) return;
    try {
      const saved = await request<AuthorityDefinition>(`/v1/games/${game.id}/authority`, { method: "PUT", body: JSON.stringify({ player_id: playerId, expected_version: authority.version, definition: draft }) });
      if (activeGameId.current !== game.id) return;
      setAuthority(saved); authorityResource.refresh(); roleResource.refresh(); setMessage(`Authority definition v${saved.version} is live.`);
    } catch (error) { setMessage((error as Error).message); throw error; }
  }

  async function createAuthorityRequest(action: string, target_unit_id: string, summary: string) {
    if (!game || !role) return;
    try { await request(`/v1/games/${game.id}/roles/${role.id}/authority-requests`, { method: "POST", body: JSON.stringify({ player_id: playerId, lease_generation: role.lease_generation, action, target_unit_id, summary }) }); setMessage("Authority request transmitted."); }
    catch (error) { setMessage((error as Error).message); }
  }

  async function decideAuthorityRequest(requestId: string, decision: "approve" | "deny") {
    if (!game || !role) return;
    try { await request(`/v1/games/${game.id}/roles/${role.id}/authority-requests/${requestId}/decision`, { method: "POST", body: JSON.stringify({ player_id: playerId, lease_generation: role.lease_generation, decision }) }); setMessage(`Request ${decision === "approve" ? "approved" : "denied"}.`); }
    catch (error) { setMessage((error as Error).message); }
  }

  function leave() {
    activeGameId.current = null; controlRequest.current?.abort(); controlPending.current = false;
    setGame(null); setRole(null); setRoles([]); setAuthority(null); setAuthorityRequests([]);
    setShowAuthority(false); setShowNetwork(false); setShowMapFilters(false); setControlError("");
    setMessage("Create a scenario or join a running game");
    void refreshLobby().catch((error: Error) => setMessage(error.message));
  }

  return <main className="app-shell">
    <header className="app-header"><span className="brand">WORLD AT WAR</span>
      <span className={"status-dot " + (playable ? connectionStatus : "")} />
      {game && game.status !== "lobby" ? <GameSessionControls game={game} isHost={game.host_player_id === playerId} pending={pendingControl} onControl={(status) => void controlGame(status)} /> : <span>{game?.status ?? "scenario lobby"}</span>}
      <span className="tick">{projection ? `TICK ${projection.tick}` : ""}</span>
      {game && <button className="secondary session-leave" onClick={leave}>Leave scenario</button>}
    </header>
    {!playable && <div className="lobby-stage"><section className="scenario-modal" aria-modal="true" role="dialog">
      <div className="modal-header"><div><h1>Scenario Command</h1><p>{message}</p></div><span className={spaceStatus?.usable ? "catalog-ready" : "catalog-missing"}>{spaceStatus?.usable ? `${spaceStatus.object_count.toLocaleString()} ORBITAL OBJECTS${spaceStatus.stale ? " · CACHED" : ""}` : "SPACE DATA REQUIRED"}</span></div>
      {!game && <>
        <div className="tabs">
          <button className={mode === "new" ? "active" : ""} onClick={() => setMode("new")}>New scenario</button>
          <button className={mode === "join" ? "active" : ""} onClick={() => setMode("join")}>Join game</button>
          <button className={`space-config-tab ${mode === "space" ? "active" : ""}`} title={!spaceStatus ? "Checking space catalog status." : spaceStatus.usable && !spaceStatus.stale ? "Recent space catalog ready." : spaceStatus.usable ? "Space catalog is stale; refresh recommended." : "Space data must be downloaded."} onClick={() => setMode("space")}>Space Configuration<CatalogTabStatus status={spaceStatus} /></button>
        </div>
        {mode === "new" && <div className="modal-body scenario-setup">
          <h2>Scenario</h2>
          {scenarios.map((scenario) => <button type="button" className={`scenario-choice ${selectedScenario?.id === scenario.id ? "selected" : ""}`} key={scenario.id} onClick={() => { setSelectedScenarioId(scenario.id); setGameTitle(scenario.title); }}><strong>{scenario.title}</strong><p>{scenario.description}</p><small>{scenario.authored_entity_count} authored entities · {scenario.role_count} roles · {scenario.requires_space_catalog ? "full public space catalog" : "no orbital catalog required"}</small></button>)}
          <label>Game title<input value={gameTitle} onChange={(event) => setGameTitle(event.target.value)} /></label>
          <button className="command" disabled={!selectedScenario || (selectedScenario.requires_space_catalog && !spaceStatus?.usable)} onClick={() => void createGame()}>Create game</button>
          {selectedScenario?.requires_space_catalog && !spaceStatus?.usable && <p className="catalog-required">A usable space catalog is required to create this scenario. <button className="text-command" onClick={() => setMode("space")}>Open Space Configuration</button></p>}
        </div>}
        {mode === "join" && <div className="modal-body"><label>Display name<input value={displayName} onChange={(event) => setDisplayName(event.target.value)} /></label><h2>Available games</h2><div className="game-list">{games.length ? games.map((item) => <button className="game-row" key={item.id} onClick={() => void selectGame(item)}><span>{item.title}</span><small>{item.status} · {item.player_roles_available} open roles</small></button>) : <p className="muted">No games have been created.</p>}</div></div>}
        {mode === "space" && <div className="modal-body space-configuration">
          <h2>Space-Track setup</h2>
          <p className="muted">Credentials are held in server memory. Remembering them stores encrypted data in an HttpOnly cookie.</p>
          {spaceStatus?.synced_unix && <CatalogDownloadTime syncedUnix={spaceStatus.synced_unix} />}
          {spaceStatus?.error && <p className="space-track-error" role="alert"><strong>{spaceStatus.using_cached_fallback ? "Refresh failed; cached catalog remains active." : "Synchronization failed."}</strong> {spaceStatus.error}</p>}
          {spaceStatus?.setup_auth_required && <label>Admin setup token<input type="password" value={adminToken} onChange={(event) => setAdminToken(event.target.value)} /></label>}
          <label>Space-Track username<input autoComplete="username" value={spaceUsername} onChange={(event) => { setSpaceUsername(event.target.value); if (spacePassword === SAVED_PASSWORD_MASK) setSpacePassword(""); setSpaceTrackFeedback(null); }} /></label>
          <label>Space-Track password<input type="password" autoComplete="current-password" value={spacePassword} onFocus={() => { if (spacePassword === SAVED_PASSWORD_MASK) setSpacePassword(""); }} onBlur={() => { if (!spacePassword && spaceStatus?.remembered_credentials && spaceUsername === spaceStatus.remembered_username) setSpacePassword(SAVED_PASSWORD_MASK); }} onChange={(event) => { setSpacePassword(event.target.value); setSpaceTrackFeedback(null); }} /></label>
          <label className="toggle"><input type="checkbox" checked={rememberCredentials} disabled={usingSavedCredentials} onChange={(event) => setRememberCredentials(event.target.checked)} />{usingSavedCredentials ? "Credentials saved for 30 days" : "Remember credentials for 30 days"}</label>
          <button className="secondary space-track-connect" aria-busy={spaceTrackSyncing} aria-describedby={catalogRefreshBlocked ? "space-track-cooldown" : undefined} disabled={catalogRefreshBlocked || spaceTrackSyncing || (spaceStatus?.setup_auth_required && !adminToken) || !spaceUsername || !spacePassword} onClick={() => void connectSpaceTrack()}>{spaceTrackSyncing ? <><span className="button-spinner" aria-hidden="true" />Authenticating and downloading…</> : catalogRefreshBlocked ? `Refresh available in ${formatRefreshWait(refreshWaitSeconds)}` : usingSavedCredentials ? "Synchronize with saved credentials" : "Connect and synchronize"}</button>
          {catalogRefreshBlocked && <p id="space-track-cooldown" className="space-track-cooldown">Catalog downloads are limited to once per hour after a successful refresh.</p>}
          {spaceTrackFeedback && <div className={`space-track-feedback ${spaceTrackFeedback.kind}`} role={spaceTrackFeedback.kind === "error" ? "alert" : "status"} aria-live="polite"><span className="feedback-icon" aria-hidden="true">{spaceTrackFeedback.kind === "success" ? "✓" : "!"}</span><span><strong>{spaceTrackFeedback.title}</strong><small>{spaceTrackFeedback.detail}</small></span></div>}
          <p className="muted">Sign-in attempts to refresh and save the catalog; a failed refresh keeps the cached catalog available.</p>
          {spaceStatus?.remembered_credentials && <button className="text-command" onClick={() => void forgetSpaceTrack()}>Forget saved credentials</button>}
        </div>}
      </>}
      {game && <div className="modal-body"><h2>{game.title}</h2><p className="muted">Claim a command role. The operational map remains offline until the scenario starts.</p><div className="role-grid">{roles.map((item) => <button key={item.id} className={`role ${role?.id === item.id ? "selected" : ""}`} disabled={item.ai_controlled || (item.held && role?.id !== item.id)} onClick={() => void claim(item)}><span>{item.name}</span><small>{item.ai_controlled ? "AI" : item.held ? "held" : item.kind.replaceAll("_", " ")}</small></button>)}</div><div className="modal-actions"><button className="secondary" onClick={leave}>Back</button>{game.host_player_id === playerId && <button className="secondary" onClick={() => setShowAuthority(true)}>Configure authorities</button>}{game.host_player_id === playerId && <button className="command" disabled={!role || pendingControl !== null} aria-busy={pendingControl !== null} onClick={() => void start()}>{pendingControl ? "Starting…" : game.status === "paused" ? "Resume scenario" : "Start scenario"}</button>}{game.host_player_id !== playerId && <span className="muted">Waiting for host to start</span>}</div></div>}
    </section></div>}
    {playable && !projection && game && <section className="workspace-loading">
      <GameSessionNotice game={game} status={connectionStatus} error={connectionError} controlError={controlError} hasProjection={false} onRetry={retryConnection} />
    </section>}
    {playable && projection && <section className="workspace">
      <aside className="sidebar"><h1>{role.name}</h1><p className="message">{game.title}</p><h2>Command</h2><button className="command" onClick={() => setShowAuthority(true)}>Authorities {authorityRequests.filter((item) => item.status.state === "pending_human" || item.status.state === "pending_external").length ? `(${authorityRequests.filter((item) => item.status.state === "pending_human" || item.status.state === "pending_external").length})` : ""}</button><button className="command network-launch" onClick={() => setShowNetwork(true)}>Network</button><button className="command map-filter-launch" onClick={() => setShowMapFilters((value) => !value)}>Map filters</button><h2>Catalog</h2><p className="muted">{game.space_catalog_enabled ? spaceStatus ? `${spaceStatus.object_count.toLocaleString()} game-pinned public objects` : "Loading catalog status" : "No orbital catalog in this scenario"}</p><p className="session-command-feedback" role="status">{message}</p></aside>
      <section className="map-region">
        <GameSessionNotice game={game} status={connectionStatus} error={connectionError} controlError={controlError} hasProjection={true} onRetry={retryConnection} />
        <Globe key={`${game.id}:${role.id}:${role.lease_generation}`} projection={projection} filters={mapFilters} gameId={game.id} playerId={playerId} roleId={role.id} spaceCatalogEnabled={game.space_catalog_enabled} keyboardEnabled={!showAuthority && !showNetwork && !showMapFilters} renderingEnabled={!showAuthority && !showNetwork} />
        {showMapFilters && <MapFilterDialog filters={mapFilters} spaceAssetsAvailable={game.space_catalog_enabled} onChange={setMapFilters} onClose={() => setShowMapFilters(false)} />}
        <div className="map-caption">{role.name} · {role.side} · operational picture</div>
      </section>
      <OperationalInspector projection={projection} role={role} canIssueOrders={canIssueOrders} onTurnNorth={() => void turnNorth()} onInspectNetwork={() => setShowNetwork(true)} />
    </section>}
    {showAuthority && authority && game && <Suspense fallback={<div className="authority-loading">Loading authority graph…</div>}><AuthorityWorkspace definition={authority} runtimeRoles={roles} units={authorityUnits} requests={authorityRequests} currentRole={role} isHost={game.host_player_id === playerId} tick={projection?.tick ?? 0} onClose={() => setShowAuthority(false)} onSave={saveAuthority} onCreateRequest={createAuthorityRequest} onDecision={decideAuthorityRequest} /></Suspense>}
    {showNetwork && game && role && <Suspense fallback={<div className="authority-loading">Loading C2 network…</div>}><NetworkWorkspace key={`${game.id}:${role.id}`} apiBase={API_BASE} gameId={game.id} playerId={playerId} roleId={role.id} onClose={() => setShowNetwork(false)} /></Suspense>}
  </main>;
}

createRoot(document.getElementById("root")!).render(<App />);

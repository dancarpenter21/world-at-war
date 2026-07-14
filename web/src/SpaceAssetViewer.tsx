import { useEffect, useMemo, useRef, useState } from "react";
import {
  Cartesian2, Cartesian3, Color, EllipsoidTerrainProvider, ImageryLayer, OpenStreetMapImageryProvider,
  Matrix4, PointPrimitive, PointPrimitiveCollection, ScreenSpaceEventHandler, ScreenSpaceEventType, Viewer,
} from "cesium";

const API_BASE = import.meta.env.VITE_API_BASE ?? "";
type FacetValue = { value: string; count: number };
type Authority = { authority_id: string; display_name: string; organization: string; kind: string; confidence: string; commandable: boolean; allowed_request_types: string[] };
type RecordEntry = { norad_catalog_id: number; cospar_id?: string; canonical_name: string; aliases: string[]; nation: string; object_type: string; orbital_regime: string; operational_status: string; operator: string; mission_category: string; launch_year?: number; radar_size_class: string; inclination_deg?: number; authority: Authority };
type Detail = { catalog_checksum: string; manifest_version?: string; enrichment_available: boolean; record: RecordEntry; raw_orbital_fields: Record<string, unknown>; markdown: string; sources: { id: string; title: string; url: string; license: string }[]; authority: Authority; confidence: string };
type Filters = { query: string; mode: "payloads" | "all"; debris: boolean; rocketBodies: boolean; facets: Record<string, string[]> };

const facetLabels: Record<string, string> = { nation: "Nation", object_type: "Object type", orbital_regime: "Orbital regime", operational_status: "Status", operator: "Operator", mission_category: "Mission", launch_year: "Launch year", radar_size_class: "Radar size", inclination_band: "Inclination", authority_kind: "Authority", commandability: "Commandability" };
const objectColors: Record<string, Color> = { PAYLOAD: Color.fromCssColorString("#58c9e8"), DEBRIS: Color.fromCssColorString("#7c858b"), "ROCKET BODY": Color.fromCssColorString("#d5a45b"), UNKNOWN: Color.fromCssColorString("#b98ad8") };

export function SpaceAssetViewer({ gameId, playerId, roleId, leaseGeneration, onClose, onMessage }: { gameId: string; playerId: string; roleId: string; leaseGeneration: number; onClose: () => void; onMessage: (message: string) => void }) {
  const globeHost = useRef<HTMLDivElement>(null);
  const viewerRef = useRef<Viewer | null>(null);
  const pointsRef = useRef<PointPrimitiveCollection | null>(null);
  const pointMap = useRef(new Map<number, PointPrimitive>());
  const workerRef = useRef<Worker | null>(null);
  const selectedPosition = useRef<Cartesian3 | null>(null);
  const selectedRef = useRef<number | undefined>(undefined);
  const followRef = useRef(false);
  const [records, setRecords] = useState<RecordEntry[]>([]);
  const recordsById = useMemo(() => new Map(records.map((record) => [record.norad_catalog_id, record])), [records]);
  const [facets, setFacets] = useState<Record<string, FacetValue[]>>({});
  const [facetCounts, setFacetCounts] = useState<Record<string, Record<string, number>>>({});
  const [visibleIds, setVisibleIds] = useState<number[]>([]);
  const [filters, setFilters] = useState<Filters>({ query: "", mode: "payloads", debris: false, rocketBodies: false, facets: {} });
  const [filterOpen, setFilterOpen] = useState(true);
  const [selected, setSelected] = useState<number>();
  const [detail, setDetail] = useState<Detail>();
  const [loadingDetail, setLoadingDetail] = useState(false);
  const [clockMs, setClockMs] = useState(Date.now());
  const [playing, setPlaying] = useState(true);
  const [speed, setSpeed] = useState(1);
  const [follow, setFollow] = useState(false);
  const [summary, setSummary] = useState("Request satellite support for joint-force operations");
  const [metadata, setMetadata] = useState({ checksum: "", manifest: "", enriched: false });

  useEffect(() => {
    if (!globeHost.current) return;
    const viewer = new Viewer(globeHost.current, { animation: false, baseLayer: new ImageryLayer(new OpenStreetMapImageryProvider({ url: "https://tile.openstreetmap.org/", credit: "OpenStreetMap contributors" })), baseLayerPicker: false, fullscreenButton: false, geocoder: false, homeButton: false, infoBox: false, navigationHelpButton: false, sceneModePicker: false, selectionIndicator: false, terrainProvider: new EllipsoidTerrainProvider(), timeline: false });
    viewer.scene.globe.baseColor = Color.fromCssColorString("#101c24");
    viewer.camera.setView({ destination: Cartesian3.fromDegrees(-35, 24, 30_000_000) });
    const points = viewer.scene.primitives.add(new PointPrimitiveCollection());
    pointsRef.current = points;
    const handler = new ScreenSpaceEventHandler(viewer.scene.canvas);
    handler.setInputAction((event: { position: Cartesian2 }) => {
      const picked = viewer.scene.pick(event.position) as { id?: number } | undefined;
      if (typeof picked?.id === "number") setSelected(picked.id);
    }, ScreenSpaceEventType.LEFT_CLICK);
    viewerRef.current = viewer;
    return () => { handler.destroy(); viewer.destroy(); viewerRef.current = null; pointsRef.current = null; pointMap.current.clear(); };
  }, []);

  useEffect(() => {
    const worker = new Worker(new URL("./spaceAssetWorker.ts", import.meta.url), { type: "module" });
    workerRef.current = worker;
    worker.onmessage = (event) => {
      const message = event.data;
      if (message.type === "error") onMessage(message.message);
      if (message.type === "ready") {
        setRecords(message.records); setFacets(message.facets);
        setMetadata({ checksum: message.checksum, manifest: message.manifestVersion ?? "baseline", enriched: message.enrichmentAvailable });
        const collection = pointsRef.current;
        if (collection) for (const record of message.records as RecordEntry[]) {
          const point = collection.add({ id: record.norad_catalog_id, position: Cartesian3.ZERO, pixelSize: record.authority.commandable ? 6 : 3, color: record.authority.commandable ? Color.fromCssColorString("#6ef2b0") : (objectColors[record.object_type] ?? objectColors.UNKNOWN), show: false });
          pointMap.current.set(record.norad_catalog_id, point);
        }
      }
      if (message.type === "filtered") {
        const next = new Set<number>(message.ids); setVisibleIds(message.ids); setFacetCounts(message.facetCounts);
        for (const [id, point] of pointMap.current) point.show = next.has(id);
      }
      if (message.type === "positions") {
        const ids = message.ids as Uint32Array; const positions = message.positions as Float64Array;
        for (let index = 0; index < ids.length; index++) {
          const position = new Cartesian3(positions[index * 3], positions[index * 3 + 1], positions[index * 3 + 2]);
          const point = pointMap.current.get(ids[index]); if (point) point.position = position;
          if (ids[index] === selectedRef.current) selectedPosition.current = position;
        }
        if (followRef.current && selectedPosition.current) viewerRef.current?.camera.lookAt(selectedPosition.current, new Cartesian3(0, -2_000_000, 800_000));
      }
      if (message.type === "orbitPath") {
        const viewer = viewerRef.current; if (!viewer) return;
        viewer.entities.removeById("selected-orbit");
        const values = message.positions as Float64Array; const positions: Cartesian3[] = [];
        for (let index = 0; index < values.length; index += 3) positions.push(new Cartesian3(values[index], values[index + 1], values[index + 2]));
        viewer.entities.add({ id: "selected-orbit", polyline: { positions, width: 2, material: Color.fromCssColorString("#f3d168") } });
      }
    };
    worker.postMessage({ type: "init", apiBase: API_BASE, gameId, playerId, roleId });
    return () => { worker.terminate(); workerRef.current = null; };
  }, [gameId, onMessage, playerId, roleId]);

  useEffect(() => { workerRef.current?.postMessage({ type: "filter", filters }); }, [filters]);
  useEffect(() => {
    const previous = selectedRef.current;
    if (previous !== undefined) {
      const record = recordsById.get(previous); const point = pointMap.current.get(previous);
      if (record && point) { point.color = record.authority.commandable ? Color.fromCssColorString("#6ef2b0") : (objectColors[record.object_type] ?? objectColors.UNKNOWN); point.pixelSize = record.authority.commandable ? 6 : 3; }
    }
    selectedRef.current = selected;
    if (selected !== undefined) { const point = pointMap.current.get(selected); if (point) { point.color = Color.fromCssColorString("#ffd86a"); point.pixelSize = 10; point.show = true; } }
  }, [recordsById, selected]);
  useEffect(() => { followRef.current = follow; if (!follow) viewerRef.current?.camera.lookAtTransform(Matrix4.IDENTITY); }, [follow]);
  useEffect(() => {
    if (selected === undefined) return;
    workerRef.current?.postMessage({ type: "select", noradId: selected }); setLoadingDetail(true); setDetail(undefined);
    const query = new URLSearchParams({ player_id: playerId, role_id: roleId });
    fetch(`${API_BASE}/v1/games/${gameId}/space-assets/${selected}?${query}`, { credentials: "include" }).then(async (response) => {
      if (!response.ok) throw new Error((await response.json()).error ?? response.statusText);
      return response.json() as Promise<Detail>;
    }).then(setDetail).catch((error: Error) => onMessage(error.message)).finally(() => setLoadingDetail(false));
  }, [gameId, onMessage, playerId, roleId, selected]);
  useEffect(() => {
    if (!playing) return;
    const timer = window.setInterval(() => setClockMs((value) => value + 1_000 * speed), 1_000);
    return () => window.clearInterval(timer);
  }, [playing, speed]);
  useEffect(() => { workerRef.current?.postMessage({ type: "clock", simulatedMs: clockMs }); }, [clockMs]);

  const visibleRecords = useMemo(() => visibleIds.slice(0, 100).map((id) => recordsById.get(id)).filter((value): value is RecordEntry => Boolean(value)), [recordsById, visibleIds]);
  const updateFacet = (facet: string, value: string, checked: boolean) => setFilters((current) => ({ ...current, facets: { ...current.facets, [facet]: checked ? [...(current.facets[facet] ?? []), value] : (current.facets[facet] ?? []).filter((item) => item !== value) } }));
  const clearFilters = () => setFilters({ query: "", mode: "payloads", debris: false, rocketBodies: false, facets: {} });

  async function submitRequest(action: string) {
    if (selected === undefined) return;
    const response = await fetch(`${API_BASE}/v1/games/${gameId}/roles/${roleId}/space-assets/${selected}/requests`, { method: "POST", credentials: "include", headers: { "content-type": "application/json" }, body: JSON.stringify({ player_id: playerId, lease_generation: leaseGeneration, action, summary }) });
    const body = await response.json();
    if (!response.ok) { onMessage(body.error ?? response.statusText); return; }
    onMessage(`Satellite authority request ${body.request_id} created.`);
  }

  return <section className="space-workspace">
    <header className="space-header"><button className="secondary" onClick={onClose}>← Operations</button><strong>SPACE ASSET VIEWER</strong><span>{visibleIds.length.toLocaleString()} / {records.length.toLocaleString()} objects</span><small>{metadata.enriched ? `cards ${metadata.manifest}` : "baseline cards"} · {metadata.checksum.slice(0, 10)}</small></header>
    <div className="space-toolbar"><input aria-label="Search space catalog" placeholder="Search name, NORAD, COSPAR, alias, operator, or mission" value={filters.query} onChange={(event) => setFilters((current) => ({ ...current, query: event.target.value }))} /><button className={filterOpen ? "active" : ""} onClick={() => setFilterOpen((value) => !value)}>Filters</button><button className={filters.mode === "payloads" ? "active" : ""} onClick={() => setFilters((current) => ({ ...current, mode: "payloads" }))}>Payloads</button><button className={filters.mode === "all" ? "active" : ""} onClick={() => setFilters((current) => ({ ...current, mode: "all" }))}>All objects</button><label><input type="checkbox" checked={filters.debris} onChange={(event) => setFilters((current) => ({ ...current, mode: "all", debris: event.target.checked }))} />Debris</label><label><input type="checkbox" checked={filters.rocketBodies} onChange={(event) => setFilters((current) => ({ ...current, mode: "all", rocketBodies: event.target.checked }))} />Rocket bodies</label></div>
    <div className={`space-body ${filterOpen ? "with-filters" : ""} ${selected !== undefined ? "with-detail" : ""}`}>
      {filterOpen && <aside className="space-filters"><div><strong>FACETS</strong><button onClick={clearFilters}>Reset</button></div>{Object.entries(facetLabels).map(([facet, label]) => <details key={facet} open={["nation", "orbital_regime", "authority_kind"].includes(facet)}><summary>{label}</summary>{(facets[facet] ?? []).slice(0, 18).map((entry) => <label key={entry.value}><input type="checkbox" checked={(filters.facets[facet] ?? []).includes(entry.value)} onChange={(event) => updateFacet(facet, entry.value, event.target.checked)} /><span>{entry.value}</span><small>{facetCounts[facet]?.[entry.value] ?? entry.count}</small></label>)}</details>)}</aside>}
      <div className="space-globe" ref={globeHost}><div className="space-results">{filters.query && visibleRecords.map((record) => <button key={record.norad_catalog_id} onClick={() => setSelected(record.norad_catalog_id)}><strong>{record.canonical_name}</strong><small>NORAD {record.norad_catalog_id} · {record.orbital_regime.toUpperCase()}</small></button>)}</div></div>
      {selected !== undefined && <aside className="space-detail"><button className="detail-close" onClick={() => { setSelected(undefined); setDetail(undefined); }}>×</button>{loadingDetail && <p>Loading card…</p>}{detail && <><div className="asset-title"><small>{detail.record.object_type} · {detail.record.orbital_regime.toUpperCase()}</small><h1>{detail.record.canonical_name}</h1><span>NORAD {selected} · {detail.record.cospar_id ?? "No COSPAR ID"}</span></div><div className="detail-actions"><button onClick={() => selectedPosition.current && viewerRef.current?.camera.flyTo({ destination: selectedPosition.current })}>Focus</button><button className={follow ? "active" : ""} onClick={() => setFollow((value) => !value)}>Follow</button></div><section className="live-elements"><h2>Live pinned orbital data</h2>{["EPOCH", "APOAPSIS", "PERIAPSIS", "PERIOD", "INCLINATION", "ECCENTRICITY"].map((field) => <div key={field}><span>{field.replaceAll("_", " ")}</span><strong>{String(detail.raw_orbital_fields[field] ?? "Unknown")}</strong></div>)}</section><section className="authority-card"><h2>Command authority</h2><strong>{detail.authority.display_name}</strong><p>{detail.authority.organization} · {detail.authority.kind.replaceAll("_", " ")} · {detail.authority.confidence}</p>{detail.authority.commandable ? <><textarea value={summary} maxLength={500} onChange={(event) => setSummary(event.target.value)} /><button className="command" onClick={() => void submitRequest("request_satellite_service")}>Request service</button><button className="secondary" onClick={() => void submitRequest("coordinate_satellite_maneuver")}>Coordinate maneuver</button></> : <p className="unresolved">Not commandable: reviewed public authority is unresolved or outside v1 scope.</p>}</section><Markdown markdown={detail.markdown} /><section><h2>Provenance</h2>{detail.sources.map((source) => <a href={source.url} target="_blank" rel="noreferrer" key={source.id}>{source.title} · {source.license}</a>)}</section></>}</aside>}
    </div>
    <footer className="space-clock"><button onClick={() => setPlaying((value) => !value)}>{playing ? "Pause" : "Play"}</button><label>Speed<select value={speed} onChange={(event) => setSpeed(Number(event.target.value))}><option value={0.25}>0.25×</option><option value={1}>1×</option><option value={10}>10×</option><option value={60}>60×</option></select></label><strong>{new Date(clockMs).toISOString().replace(".000", "")} UTC</strong><input aria-label="Time scrub" type="range" min={-86400} max={86400} step={60} value={Math.max(-86400, Math.min(86400, Math.round((clockMs - Date.now()) / 1000)))} onChange={(event) => { setPlaying(false); setClockMs(Date.now() + Number(event.target.value) * 1_000); }} /><button onClick={() => { setClockMs(Date.now()); setPlaying(true); }}>Now</button></footer>
  </section>;
}

function Markdown({ markdown }: { markdown: string }) {
  const body = markdown.replace(/^---[\s\S]*?---\s*/, "");
  return <section className="markdown-card">{body.split("\n").map((line, index) => {
    if (line.startsWith("# ")) return <h1 key={index}>{line.slice(2)}</h1>;
    if (line.startsWith("## ")) return <h2 key={index}>{line.slice(3)}</h2>;
    if (line.startsWith("- ")) return <p className="source-line" key={index}>• <SafeInline value={line.slice(2)} /></p>;
    return line ? <p key={index}><SafeInline value={line} /></p> : null;
  })}</section>;
}

function SafeInline({ value }: { value: string }) {
  const match = value.match(/^\[([^\]]+)]\((https:\/\/[^)]+)\)(.*)$/);
  if (match) return <><a href={match[2]} target="_blank" rel="noreferrer">{match[1]}</a>{match[3]}</>;
  return <>{value.replaceAll("**", "")}</>;
}

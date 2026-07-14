/// <reference lib="webworker" />

import { eciToEcf, gstime, json2satrec, propagate, type SatRec } from "satellite.js";

type Authority = { kind: string; commandable: boolean };
type RecordEntry = {
  norad_catalog_id: number; cospar_id?: string; canonical_name: string; aliases: string[];
  nation: string; object_type: string; orbital_regime: string; operational_status: string;
  operator: string; mission_category: string; launch_year?: number; radar_size_class: string;
  inclination_deg?: number; authority: Authority;
};
type Filters = { query: string; mode: "payloads" | "all"; debris: boolean; rocketBodies: boolean; facets: Record<string, string[]> };
type Orbital = { id: number; satrec: SatRec };

const scope = self as unknown as DedicatedWorkerGlobalScope;
let records: RecordEntry[] = [];
let orbital: Orbital[] = [];
let orbitalById = new Map<number, SatRec>();
let visible = new Set<number>();
let selected: number | undefined;
let simulatedMs = Date.now();
let propagationTimer: number | undefined;
const facetFields = ["nation", "object_type", "orbital_regime", "operational_status", "operator", "mission_category", "launch_year", "radar_size_class", "inclination_band", "authority_kind", "commandability"];

scope.onmessage = (event: MessageEvent) => {
  const message = event.data;
  if (message.type === "init") void initialize(message);
  if (message.type === "filter") applyFilters(message.filters as Filters);
  if (message.type === "clock") { simulatedMs = message.simulatedMs; propagateVisible(); }
  if (message.type === "select") { selected = message.noradId; postOrbitPath(); }
};

async function initialize(message: { apiBase: string; gameId: string; playerId: string; roleId: string }) {
  const query = new URLSearchParams({ player_id: message.playerId, role_id: message.roleId });
  const [indexResponse, rawResponse] = await Promise.all([
    fetch(`${message.apiBase}/v1/games/${message.gameId}/space-assets?${query}`, { credentials: "include" }),
    fetch(`${message.apiBase}/v1/games/${message.gameId}/space-catalog?${query}`, { credentials: "include" }),
  ]);
  if (!indexResponse.ok || !rawResponse.ok) {
    scope.postMessage({ type: "error", message: "Unable to load the game-pinned space workspace." });
    return;
  }
  const index = await indexResponse.json();
  const raw = await rawResponse.json();
  records = index.records;
  orbital = raw.objects.flatMap((value: Record<string, unknown>) => {
    try {
      const id = Number(value.NORAD_CAT_ID);
      const satrec = json2satrec(value as never);
      return Number.isFinite(id) && satrec.error === 0 ? [{ id, satrec }] : [];
    } catch { return []; }
  });
  orbitalById = new Map(orbital.map((value) => [value.id, value.satrec]));
  scope.postMessage({ type: "ready", records, facets: index.facets, checksum: index.catalog_checksum, manifestVersion: index.manifest_version, enrichmentAvailable: index.enrichment_available });
  applyFilters({ query: "", mode: "payloads", debris: false, rocketBodies: false, facets: {} });
  propagationTimer = scope.setInterval(propagateVisible, 10_000);
}

function applyFilters(filters: Filters) {
  const query = filters.query.trim().toLocaleLowerCase();
  const result = records.filter((record) => {
    if (filters.mode === "payloads" && record.object_type !== "PAYLOAD") return false;
    if (filters.mode === "all") {
      if (!filters.debris && record.object_type === "DEBRIS") return false;
      if (!filters.rocketBodies && record.object_type === "ROCKET BODY") return false;
    }
    if (query && !searchText(record).includes(query)) return false;
    return Object.entries(filters.facets).every(([facet, selectedValues]) => !selectedValues.length || selectedValues.includes(facetValue(record, facet)));
  });
  visible = new Set(result.map((record) => record.norad_catalog_id));
  const facetCounts: Record<string, Record<string, number>> = {};
  for (const record of result) {
    for (const facet of facetFields) {
      const value = facetValue(record, facet);
      facetCounts[facet] ??= {};
      facetCounts[facet][value] = (facetCounts[facet][value] ?? 0) + 1;
    }
  }
  scope.postMessage({ type: "filtered", ids: result.map((record) => record.norad_catalog_id), count: result.length, facetCounts });
  propagateVisible();
}

function searchText(record: RecordEntry) {
  return [record.canonical_name, record.norad_catalog_id, record.cospar_id, ...record.aliases, record.operator, record.mission_category].join(" ").toLocaleLowerCase();
}

function facetValue(record: RecordEntry, facet: string): string {
  if (facet === "authority_kind") return record.authority.kind;
  if (facet === "commandability") return record.authority.commandable ? "commandable" : "not_commandable";
  if (facet === "inclination_band") {
    const value = record.inclination_deg;
    if (value === undefined) return "Unknown";
    return value < 10 ? "0–10°" : value < 45 ? "10–45°" : value < 70 ? "45–70°" : value < 100 ? "70–100°" : "100–180°";
  }
  const value = record[facet as keyof RecordEntry];
  return value === undefined ? "Unknown" : String(value);
}

function propagateVisible() {
  if (!orbital.length) return;
  const ids: number[] = [];
  const positions: number[] = [];
  const date = new Date(simulatedMs);
  const gmst = gstime(date);
  for (const value of orbital) {
    if (!visible.has(value.id) && value.id !== selected) continue;
    const result = propagate(value.satrec, date);
    if (!result || !result.position || typeof result.position === "boolean") continue;
    const earthFixed = eciToEcf(result.position, gmst);
    ids.push(value.id);
    positions.push(earthFixed.x * 1_000, earthFixed.y * 1_000, earthFixed.z * 1_000);
  }
  const idBuffer = Uint32Array.from(ids);
  const positionBuffer = Float64Array.from(positions);
  scope.postMessage({ type: "positions", ids: idBuffer, positions: positionBuffer }, [idBuffer.buffer, positionBuffer.buffer]);
}

function postOrbitPath() {
  if (selected === undefined) return;
  const satrec = orbitalById.get(selected);
  if (!satrec) return;
  const positions: number[] = [];
  for (let offset = -90; offset <= 90; offset += 3) {
    const date = new Date(simulatedMs + offset * 60_000);
    const result = propagate(satrec, date);
    if (!result || !result.position || typeof result.position === "boolean") continue;
    const earthFixed = eciToEcf(result.position, gstime(date));
    positions.push(earthFixed.x * 1_000, earthFixed.y * 1_000, earthFixed.z * 1_000);
  }
  const buffer = Float64Array.from(positions);
  scope.postMessage({ type: "orbitPath", noradId: selected, positions: buffer }, [buffer.buffer]);
}

export {};

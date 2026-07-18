import {
  BillboardCollection,
  Cartesian2,
  Cartesian3,
  ConstantProperty,
  NearFarScalar,
  ScreenSpaceEventHandler,
  ScreenSpaceEventType,
  type Viewer
} from "cesium";

export type AirportSummary = {
  id: string;
  name: string;
  kind: string;
  country_code: string;
  region_code?: string;
  municipality?: string;
  military_use: string;
  latitude_deg: number;
  longitude_deg: number;
  runway_count: number;
  longest_runway_m?: number;
};

export type AirportListResponse = {
  checksum: string;
  total: number;
  limit: number;
  offset: number;
  airports: AirportSummary[];
};

type AirportIdentifiers = {
  ourairports_ident?: string;
  icao?: string;
  iata?: string;
  gps?: string;
  local?: string;
  faa_site_number?: string;
  faa_airport_id?: string;
};

type PavementClassification = {
  system: "acn_pcn" | "acr_pcr";
  value: number;
  pavement_type?: string;
  subgrade_strength?: string;
  tire_pressure?: string;
  determination_method?: string;
};

type GrossWeightLimits = {
  single_wheel_kg?: number;
  dual_wheel_kg?: number;
  dual_tandem_kg?: number;
  double_dual_tandem_kg?: number;
};

export type AirportDetail = Omit<AirportSummary, "runway_count" | "longest_runway_m"> & {
  status: string;
  elevation_m?: number;
  identifiers: AirportIdentifiers;
  ownership_type?: string;
  facility_use?: string;
  joint_use?: boolean;
  military_landing_rights?: boolean;
  runways: {
    id: string;
    designator: string;
    length_m?: number;
    width_m?: number;
    surface: string;
    surface_raw?: string;
    status: string;
    lighted?: boolean;
    condition?: string;
    pavement?: PavementClassification;
    gross_weight_limits: GrossWeightLimits;
  }[];
};

const AIRPORT_PREFIX = "airport:";
const AIRPORT_DETAIL_ID = "selected-airport-details";
let airportIcon: HTMLCanvasElement | undefined;

export function airportIconCanvas() {
  if (airportIcon) return airportIcon;
  const canvas = document.createElement("canvas");
  canvas.width = 24;
  canvas.height = 24;
  const context = canvas.getContext("2d");
  if (!context) throw new Error("airport icon canvas is unavailable");

  context.lineCap = "round";
  context.lineJoin = "round";
  for (const [strokeStyle, lineWidth] of [["rgba(255,255,255,0.92)", 5], ["#172b34", 2.5]] as const) {
    context.beginPath();
    context.moveTo(4, 20);
    context.lineTo(20, 4);
    context.moveTo(7, 5);
    context.lineTo(19, 17);
    context.strokeStyle = strokeStyle;
    context.lineWidth = lineWidth;
    context.stroke();
  }
  airportIcon = canvas;
  return canvas;
}

function escapeHtml(value: unknown) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function metres(value: number | undefined) {
  if (value === undefined) return "Not reported";
  return `${Math.round(value).toLocaleString()} m / ${Math.round(value * 3.28084).toLocaleString()} ft`;
}

function weight(value: number | undefined) {
  if (value === undefined) return "Not reported";
  return `${Math.round(value).toLocaleString()} kg / ${Math.round(value * 2.20462).toLocaleString()} lb`;
}

function pavementRating(pavement: PavementClassification | undefined) {
  if (!pavement) return "Not reported";
  const system = pavement.system === "acr_pcr" ? "ACR/PCR" : "ACN/PCN";
  const attributes = [pavement.pavement_type, pavement.subgrade_strength, pavement.tire_pressure, pavement.determination_method]
    .filter(Boolean)
    .join(" / ");
  return `${system} ${pavement.value}${attributes ? ` (${attributes})` : ""}`;
}

function weightRatings(limits: GrossWeightLimits) {
  const values = [
    ["Single wheel", limits.single_wheel_kg],
    ["Dual wheel", limits.dual_wheel_kg],
    ["Dual tandem", limits.dual_tandem_kg],
    ["Double dual tandem", limits.double_dual_tandem_kg]
  ].filter((entry): entry is [string, number] => typeof entry[1] === "number");
  if (!values.length) return "Not reported";
  return values.map(([label, value]) => `<div><strong>${label}:</strong> ${escapeHtml(weight(value))}</div>`).join("");
}

export function airportDescriptionHtml(airport: AirportDetail) {
  const icao = airport.identifiers.icao ?? "Not reported";
  const rows = airport.runways.length
    ? airport.runways.map((runway) => `<tr>
        <td><strong>${escapeHtml(runway.designator || "Not reported")}</strong><br><small>${escapeHtml(runway.status)}</small></td>
        <td>${escapeHtml(metres(runway.length_m))}<br><small>Width: ${escapeHtml(metres(runway.width_m))}</small></td>
        <td>${escapeHtml(runway.surface_raw ?? runway.surface)}</td>
        <td>${escapeHtml(pavementRating(runway.pavement))}</td>
        <td>${weightRatings(runway.gross_weight_limits)}</td>
      </tr>`).join("")
    : '<tr><td colspan="5">No runway records reported.</td></tr>';
  return `<div style="font-family:system-ui,sans-serif;color:#e6edf1;background:#17232b;padding:12px;min-width:720px">
    <h2 style="margin:0 0 4px">${escapeHtml(airport.name)}</h2>
    <p style="margin:0 0 12px"><strong>ICAO:</strong> ${escapeHtml(icao)} &nbsp; <strong>Country:</strong> ${escapeHtml(airport.country_code)} &nbsp; <strong>Use:</strong> ${escapeHtml(airport.military_use.replaceAll("_", " "))}</p>
    <table style="width:100%;border-collapse:collapse;font-size:13px">
      <thead><tr><th>Runway</th><th>Length / width</th><th>Surface</th><th>Pavement rating</th><th>Reported gross-weight limits</th></tr></thead>
      <tbody>${rows}</tbody>
    </table>
    <p style="font-size:11px;color:#aebdc5">Weight limits are tied to landing-gear configuration. “Not reported” does not mean unrestricted.</p>
  </div>`;
}

export class AirportLayer {
  private readonly billboards = new BillboardCollection();
  private readonly clickHandler: ScreenSpaceEventHandler;
  private selectionGeneration = 0;

  constructor(
    private readonly viewer: Viewer,
    private readonly loadAirport: (airportId: string) => Promise<AirportDetail>
  ) {
    viewer.scene.primitives.add(this.billboards);
    this.clickHandler = new ScreenSpaceEventHandler(viewer.scene.canvas);
    this.clickHandler.setInputAction((event: { position: Cartesian2 }) => {
      const picked = viewer.scene.pick(event.position) as { id?: unknown } | undefined;
      if (typeof picked?.id !== "string" || !picked.id.startsWith(AIRPORT_PREFIX)) return;
      void this.select(picked.id.slice(AIRPORT_PREFIX.length));
    }, ScreenSpaceEventType.LEFT_CLICK);
  }

  update(airports: AirportSummary[]) {
    const image = airportIconCanvas();
    this.billboards.removeAll();
    for (const airport of airports) {
      this.billboards.add({
        id: `${AIRPORT_PREFIX}${airport.id}`,
        image,
        position: Cartesian3.fromDegrees(airport.longitude_deg, airport.latitude_deg, 20),
        scaleByDistance: new NearFarScalar(100_000, 1.1, 20_000_000, 0.65)
      });
    }
  }

  private async select(airportId: string) {
    const generation = ++this.selectionGeneration;
    this.viewer.entities.removeById(AIRPORT_DETAIL_ID);
    const loading = this.viewer.entities.add({
      id: AIRPORT_DETAIL_ID,
      name: "Loading airport details…",
      description: "Loading runway records…"
    });
    this.viewer.selectedEntity = loading;
    try {
      const airport = await this.loadAirport(airportId);
      if (generation !== this.selectionGeneration) return;
      this.viewer.entities.removeById(AIRPORT_DETAIL_ID);
      const detail = this.viewer.entities.add({
        id: AIRPORT_DETAIL_ID,
        name: `${airport.name} (${airport.identifiers.icao ?? "no ICAO"})`,
        position: Cartesian3.fromDegrees(airport.longitude_deg, airport.latitude_deg, airport.elevation_m ?? 20),
        description: airportDescriptionHtml(airport)
      });
      this.viewer.selectedEntity = detail;
    } catch (error) {
      if (generation !== this.selectionGeneration) return;
      loading.name = "Airport details unavailable";
      loading.description = new ConstantProperty(escapeHtml(error instanceof Error ? error.message : "The airport record could not be loaded."));
    }
  }

  destroy() {
    this.selectionGeneration++;
    this.clickHandler.destroy();
    this.viewer.entities.removeById(AIRPORT_DETAIL_ID);
    if (!this.billboards.isDestroyed()) {
      this.viewer.scene.primitives.remove(this.billboards);
    }
  }
}

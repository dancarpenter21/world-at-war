import {
  BillboardCollection,
  Cartesian3,
  NearFarScalar,
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

export class AirportLayer {
  private readonly billboards = new BillboardCollection();

  constructor(private readonly viewer: Viewer) {
    viewer.scene.primitives.add(this.billboards);
  }

  update(airports: AirportSummary[]) {
    const image = airportIconCanvas();
    this.billboards.removeAll();
    for (const airport of airports) {
      this.billboards.add({
        id: `airport:${airport.id}`,
        image,
        position: Cartesian3.fromDegrees(airport.longitude_deg, airport.latitude_deg, 20),
        scaleByDistance: new NearFarScalar(100_000, 1.1, 20_000_000, 0.65)
      });
    }
  }

  destroy() {
    if (!this.billboards.isDestroyed()) {
      this.viewer.scene.primitives.remove(this.billboards);
    }
  }
}

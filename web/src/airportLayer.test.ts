import { describe, expect, it } from "vitest";
import { airportDescriptionHtml, type AirportDetail } from "./airportLayer";

const airport: AirportDetail = {
  id: "ourairports:1",
  name: "Test & Field",
  kind: "large_airport",
  status: "open",
  country_code: "US",
  military_use: "joint",
  latitude_deg: 38.0,
  longitude_deg: -77.0,
  identifiers: { icao: "KTST" },
  runways: [{
    id: "runway:1",
    designator: "09/27",
    length_m: 3_048,
    width_m: 45.72,
    surface: "concrete",
    status: "open",
    pavement: { system: "acr_pcr", value: 72 },
    gross_weight_limits: { dual_wheel_kg: 80_000 }
  }]
};

describe("airportDescriptionHtml", () => {
  it("renders identifying data and all runway ratings in useful units", () => {
    const html = airportDescriptionHtml(airport);
    expect(html).toContain("Test &amp; Field");
    expect(html).toContain("KTST");
    expect(html).toContain("09/27");
    expect(html).toContain("3,048 m / 10,000 ft");
    expect(html).toContain("ACR/PCR 72");
    expect(html).toContain("80,000 kg / 176,370 lb");
  });

  it("labels missing ratings as not reported and escapes provider text", () => {
    const html = airportDescriptionHtml({
      ...airport,
      name: "<script>alert(1)</script>",
      identifiers: {},
      runways: [{ ...airport.runways[0], pavement: undefined, gross_weight_limits: {} }]
    });
    expect(html).not.toContain("<script>");
    expect(html).toContain("&lt;script&gt;");
    expect(html).toContain("Not reported");
  });
});

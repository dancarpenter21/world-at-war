import { Cartesian3, Cartographic, ConstantPositionProperty, JulianDate } from "cesium";
import { expect, test } from "vitest";
import { SmoothPositions } from "./smoothPositions";

test("received movement crosses the date line smoothly above the surface and stops on pause", () => {
  let now = 0;
  const smoothing = new SmoothPositions(() => now);
  const start = Cartesian3.fromDegrees(179.999, 0, 10), end = Cartesian3.fromDegrees(-179.999, 0, 20);
  const position = new ConstantPositionProperty(start);
  smoothing.setRunning(true); smoothing.update("unit", position, start);
  now = 1000; smoothing.update("unit", position, end);
  expect(Cartesian3.equals(position.getValue(JulianDate.now())!, start)).toBe(true);
  now = 1500; expect(smoothing.tick()).toBe(true);
  const halfway = Cartographic.fromCartesian(position.getValue(JulianDate.now())!);
  expect(Math.abs(halfway.longitude)).toBeCloseTo(Math.PI, 8);
  expect(halfway.height).toBeCloseTo(15, 5);
  smoothing.setRunning(false);
  now = 5000; expect(smoothing.tick()).toBe(false);
  expect(Cartesian3.equals(position.getValue(JulianDate.now())!, end)).toBe(true);
});

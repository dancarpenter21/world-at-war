import { Cartographic, Ellipsoid, EllipsoidRhumbLine, Math as CesiumMath, type Camera } from "cesium";
import { describe, expect, it, vi } from "vitest";
import { isEditableTarget, MapKeyboardController, panSpeedForHeight } from "./mapKeyboardControls";

type ViewOptions = Parameters<Camera["setView"]>[0];

function cameraAt({
  longitude = 0,
  latitude = 0,
  height = 1_000,
  heading = 0,
  pitch = -1,
  roll = 0.1,
} = {}) {
  const setView = vi.fn<(options: ViewOptions) => void>();
  const camera = {
    positionCartographic: Cartographic.fromDegrees(longitude, latitude, height),
    heading,
    pitch,
    roll,
    setView,
  } as unknown as Camera;
  return { camera, setView };
}

function keyEvent(code: string, overrides: Partial<{
  target: EventTarget | null;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
}> = {}) {
  return {
    code,
    target: null,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    preventDefault: vi.fn(),
    ...overrides,
  };
}

function destinationCartographic(setView: ReturnType<typeof cameraAt>["setView"]) {
  const destination = setView.mock.calls.at(-1)?.[0].destination;
  if (!destination || "west" in destination) throw new Error("Expected a Cartesian destination");
  return Ellipsoid.WGS84.cartesianToCartographic(destination);
}

describe("MapKeyboardController", () => {
  it("pans in screen-relative directions while preserving altitude and camera orientation", () => {
    const { camera, setView } = cameraAt({ heading: CesiumMath.toRadians(90), height: 1_000 });
    const controller = new MapKeyboardController(camera, Ellipsoid.WGS84);
    const event = keyEvent("KeyW");

    controller.handleKeyDown(event);
    controller.step(1);

    const destination = destinationCartographic(setView);
    const options = setView.mock.calls[0][0];
    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(destination.longitude).toBeGreaterThan(0);
    expect(destination.latitude).toBeCloseTo(0, 8);
    expect(destination.height).toBeCloseTo(1_000, 5);
    expect(options.orientation).toEqual({ heading: CesiumMath.toRadians(90), pitch: -1, roll: 0.1 });
  });

  it("normalizes diagonal movement to the same distance as one direction", () => {
    const straight = cameraAt({ height: 2_000 });
    const diagonal = cameraAt({ height: 2_000 });
    const straightController = new MapKeyboardController(straight.camera, Ellipsoid.WGS84);
    const diagonalController = new MapKeyboardController(diagonal.camera, Ellipsoid.WGS84);

    straightController.handleKeyDown(keyEvent("KeyW"));
    diagonalController.handleKeyDown(keyEvent("KeyW"));
    diagonalController.handleKeyDown(keyEvent("KeyD"));
    straightController.step(0.1);
    diagonalController.step(0.1);

    const start = Cartographic.fromDegrees(0, 0);
    const straightDistance = new EllipsoidRhumbLine(start, destinationCartographic(straight.setView)).surfaceDistance;
    const diagonalDistance = new EllipsoidRhumbLine(start, destinationCartographic(diagonal.setView)).surfaceDistance;
    expect(diagonalDistance).toBeCloseTo(straightDistance, 5);
  });

  it("turns Q left and E right without changing pitch or roll", () => {
    const left = cameraAt();
    const right = cameraAt();
    const leftController = new MapKeyboardController(left.camera, Ellipsoid.WGS84);
    const rightController = new MapKeyboardController(right.camera, Ellipsoid.WGS84);

    leftController.handleKeyDown(keyEvent("KeyQ"));
    rightController.handleKeyDown(keyEvent("KeyE"));
    leftController.step(0.1);
    rightController.step(0.1);

    const leftOptions = left.setView.mock.calls[0][0];
    const rightOptions = right.setView.mock.calls[0][0];
    expect(leftOptions.destination).toBeUndefined();
    expect(rightOptions.destination).toBeUndefined();
    expect("heading" in leftOptions.orientation! && leftOptions.orientation.heading).toBeCloseTo(CesiumMath.toRadians(354));
    expect("heading" in rightOptions.orientation! && rightOptions.orientation.heading).toBeCloseTo(CesiumMath.toRadians(6));
    expect(leftOptions.orientation).toMatchObject({ pitch: -1, roll: 0.1 });
    expect(rightOptions.orientation).toMatchObject({ pitch: -1, roll: 0.1 });
  });

  it("ignores editable targets and modifier shortcuts", () => {
    const { camera, setView } = cameraAt();
    const controller = new MapKeyboardController(camera, Ellipsoid.WGS84);
    const editableTarget = { closest: vi.fn(() => ({})) } as unknown as EventTarget;
    const editableEvent = keyEvent("KeyW", { target: editableTarget });
    const movementEvent = keyEvent("KeyD");
    const shortcutEvent = keyEvent("ControlLeft", { ctrlKey: true });

    controller.handleKeyDown(editableEvent);
    controller.handleKeyDown(movementEvent);
    controller.handleKeyDown(shortcutEvent);
    controller.step(0.1);

    expect(isEditableTarget(editableTarget)).toBe(true);
    expect(editableEvent.preventDefault).not.toHaveBeenCalled();
    expect(movementEvent.preventDefault).toHaveBeenCalledOnce();
    expect(shortcutEvent.preventDefault).not.toHaveBeenCalled();
    expect(setView).not.toHaveBeenCalled();
  });

  it("clears held movement when disabled or explicitly reset", () => {
    const { camera, setView } = cameraAt();
    const controller = new MapKeyboardController(camera, Ellipsoid.WGS84);

    controller.handleKeyDown(keyEvent("KeyW"));
    controller.setEnabled(false);
    controller.step(0.1);
    controller.setEnabled(true);
    controller.step(0.1);
    expect(setView).not.toHaveBeenCalled();

    controller.handleKeyDown(keyEvent("KeyW"));
    controller.clear();
    controller.step(0.1);
    expect(setView).not.toHaveBeenCalled();
  });

  it("scales pan speed with altitude and clamps extreme values", () => {
    expect(panSpeedForHeight(-50)).toBe(100);
    expect(panSpeedForHeight(25_000)).toBe(25_000);
    expect(panSpeedForHeight(20_000_000)).toBe(4_000_000);
  });
});

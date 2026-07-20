import {
  Cartesian3,
  Cartographic,
  Ellipsoid,
  EllipsoidRhumbLine,
  Math as CesiumMath,
  type Camera,
} from "cesium";

const CONTROL_CODES = new Set(["KeyW", "KeyA", "KeyS", "KeyD", "KeyQ", "KeyE"]);
const TURN_RATE_RADIANS_PER_SECOND = CesiumMath.toRadians(60);
const MINIMUM_PAN_SPEED_METERS_PER_SECOND = 100;
const MAXIMUM_PAN_SPEED_METERS_PER_SECOND = 4_000_000;
const MAXIMUM_FRAME_SECONDS = 0.1;

type ControlCode = "KeyW" | "KeyA" | "KeyS" | "KeyD" | "KeyQ" | "KeyE";
type KeyboardInput = Pick<KeyboardEvent, "code" | "target" | "altKey" | "ctrlKey" | "metaKey" | "preventDefault">;

export type AttachedMapKeyboardControls = {
  setEnabled(enabled: boolean): void;
  destroy(): void;
};

export function panSpeedForHeight(height: number) {
  return Math.min(
    MAXIMUM_PAN_SPEED_METERS_PER_SECOND,
    Math.max(MINIMUM_PAN_SPEED_METERS_PER_SECOND, height),
  );
}

export function isEditableTarget(target: EventTarget | null) {
  const candidate = target as { closest?: (selector: string) => Element | null } | null;
  return Boolean(candidate?.closest?.("input, textarea, select, [contenteditable=''], [contenteditable='true']"));
}

export class MapKeyboardController {
  private readonly pressed = new Set<ControlCode>();
  private enabled = true;
  private rhumbLine: EllipsoidRhumbLine | undefined;
  private readonly start = new Cartographic();
  private readonly end = new Cartographic();
  private readonly destination = new Cartesian3();

  constructor(
    private readonly camera: Camera,
    private readonly ellipsoid: Ellipsoid,
  ) {}

  setEnabled(enabled: boolean) {
    this.enabled = enabled;
    if (!enabled) this.clear();
  }

  handleKeyDown(event: KeyboardInput) {
    if (!this.enabled) return;
    if (event.altKey || event.ctrlKey || event.metaKey) {
      this.clear();
      return;
    }
    if (!isControlCode(event.code) || isEditableTarget(event.target)) return;

    this.pressed.add(event.code);
    event.preventDefault();
  }

  handleKeyUp(event: KeyboardInput) {
    if (!isControlCode(event.code)) return;
    const wasPressed = this.pressed.delete(event.code);
    if (wasPressed) event.preventDefault();
  }

  clear() {
    this.pressed.clear();
  }

  step(elapsedSeconds: number) {
    if (!this.enabled || !this.pressed.size) return;

    const frameSeconds = Math.min(MAXIMUM_FRAME_SECONDS, Math.max(0, elapsedSeconds));
    if (frameSeconds === 0) return;

    const forward = Number(this.pressed.has("KeyW")) - Number(this.pressed.has("KeyS"));
    const right = Number(this.pressed.has("KeyD")) - Number(this.pressed.has("KeyA"));
    const turn = Number(this.pressed.has("KeyE")) - Number(this.pressed.has("KeyQ"));
    if (forward === 0 && right === 0 && turn === 0) return;

    const start = Cartographic.clone(this.camera.positionCartographic, this.start);
    const pitch = this.camera.pitch;
    const roll = this.camera.roll;
    const heading = CesiumMath.zeroToTwoPi(
      this.camera.heading + turn * TURN_RATE_RADIANS_PER_SECOND * frameSeconds,
    );
    let destination: Cartesian3 | undefined;

    if (forward !== 0 || right !== 0) {
      const screenRelativeBearing = Math.atan2(right, forward);
      const distance = panSpeedForHeight(start.height) * frameSeconds;
      this.rhumbLine = EllipsoidRhumbLine.fromStartHeadingDistance(
        start,
        heading + screenRelativeBearing,
        distance,
        this.ellipsoid,
        this.rhumbLine,
      );
      const end = Cartographic.clone(this.rhumbLine.end, this.end);
      end.height = start.height;
      destination = this.ellipsoid.cartographicToCartesian(end, this.destination);
    }

    this.camera.setView({
      destination,
      orientation: { heading, pitch, roll },
    });
  }
}

export function attachMapKeyboardControls(camera: Camera, ellipsoid: Ellipsoid): AttachedMapKeyboardControls {
  const controller = new MapKeyboardController(camera, ellipsoid);
  let previousFrame = performance.now();
  let animationFrame = 0;
  let destroyed = false;

  const onKeyDown = (event: KeyboardEvent) => controller.handleKeyDown(event);
  const onKeyUp = (event: KeyboardEvent) => controller.handleKeyUp(event);
  const onBlur = () => controller.clear();
  const onFocusIn = (event: FocusEvent) => {
    if (isEditableTarget(event.target)) controller.clear();
  };
  const onVisibilityChange = () => {
    if (document.hidden) controller.clear();
  };
  const update = (timestamp: number) => {
    if (destroyed) return;
    controller.step((timestamp - previousFrame) / 1_000);
    previousFrame = timestamp;
    animationFrame = window.requestAnimationFrame(update);
  };

  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("keyup", onKeyUp);
  window.addEventListener("blur", onBlur);
  document.addEventListener("focusin", onFocusIn);
  document.addEventListener("visibilitychange", onVisibilityChange);
  animationFrame = window.requestAnimationFrame(update);

  return {
    setEnabled: (enabled) => controller.setEnabled(enabled),
    destroy: () => {
      destroyed = true;
      controller.clear();
      window.cancelAnimationFrame(animationFrame);
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("blur", onBlur);
      document.removeEventListener("focusin", onFocusIn);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    },
  };
}

function isControlCode(code: string): code is ControlCode {
  return CONTROL_CODES.has(code);
}

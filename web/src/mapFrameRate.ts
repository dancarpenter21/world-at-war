/** Map rendering is independent of the authoritative simulation's 60 Hz clock. */
export function parseMapFrameRate(value: string | undefined): number {
  const requested = Number(value);
  return Number.isInteger(requested) && requested >= 1 && requested <= 60 ? requested : 60;
}

export const MAP_FRAME_RATE = parseMapFrameRate(import.meta.env.VITE_MAP_FRAME_RATE);

export function parseMapResolutionScale(value: string | undefined): number {
  const requested = Number(value);
  return Number.isFinite(requested) && requested >= 0.25 && requested <= 1 ? requested : 1;
}
export const MAP_RESOLUTION_SCALE = parseMapResolutionScale(import.meta.env.VITE_MAP_RESOLUTION_SCALE);

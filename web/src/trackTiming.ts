import type { Track } from "./globeEntities";

/** The authoritative prototype clock advances in one-second ticks. */
export function trackTiming(track: Pick<Track, "observed_tick" | "received_tick">, tick: number) {
  const observationAge = Math.max(0, tick - track.observed_tick);
  const receiptAge = Math.max(0, tick - track.received_tick);
  const deliveryDelay = Math.max(0, track.received_tick - track.observed_tick);
  return {
    isCurrentObservation: observationAge === 0,
    observationAge, receiptAge, deliveryDelay,
    observationLabel: observationAge === 0 ? "Observed this tick" : `Observed ${formatAge(observationAge)} ago`,
    receiptLabel: receiptAge === 0 ? "Received this tick" : `Received ${formatAge(receiptAge)} ago`,
    deliveryLabel: deliveryDelay === 0 ? "Local observation" : `${formatAge(deliveryDelay)} delivery delay`
  };
}

function formatAge(seconds: number) {
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds % 60;
  return `${minutes}m ${remainder}s`;
}
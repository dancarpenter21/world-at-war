import { describe, expect, it } from "vitest";
import { trackTiming } from "./trackTiming";

describe("track observation and receipt timing", () => {
  it("labels a current local observation", () => {
    expect(trackTiming({ observed_tick: 10, received_tick: 10 }, 10)).toMatchObject({
      isCurrentObservation: true, observationAge: 0, receiptAge: 0, deliveryDelay: 0,
      observationLabel: "Observed this tick", receiptLabel: "Received this tick", deliveryLabel: "Local observation"
    });
  });

  it("keeps radio delay separate from time since receipt", () => {
    expect(trackTiming({ observed_tick: 4, received_tick: 8 }, 15)).toMatchObject({
      isCurrentObservation: false, observationAge: 11, receiptAge: 7, deliveryDelay: 4,
      observationLabel: "Observed 11s ago", receiptLabel: "Received 7s ago", deliveryLabel: "4s delivery delay"
    });
  });

  it("formats old measurements and never displays a negative age during a resync", () => {
    expect(trackTiming({ observed_tick: 1, received_tick: 2 }, 126).observationLabel).toBe("Observed 2m 5s ago");
    expect(trackTiming({ observed_tick: 11, received_tick: 15 }, 10)).toMatchObject({ observationAge: 0, receiptAge: 0, deliveryDelay: 4 });
  });
});
import { describe, expect, it } from "vitest";
import { parseSavedSession } from "./savedSession";

const selection = { player_id: "player", game_id: "game", role_id: "role", lease_generation: 3 };

describe("saved game selection", () => {
  it("retains the identity and exact lease of a valid selection", () => {
    expect(parseSavedSession(JSON.stringify(selection))).toEqual(selection);
  });
  it("ignores missing, malformed, primitive, and array values", () => {
    for (const raw of [null, "", "{", "null", "true", "42", '"session"', "[]", "{}"])
      expect(parseSavedSession(raw)).toBeNull();
  });
  it("rejects missing or empty identity fields", () => {
    for (const field of ["player_id", "game_id", "role_id"]) {
      for (const value of [undefined, null, "", 5])
        expect(parseSavedSession(JSON.stringify({ ...selection, [field]: value }))).toBeNull();
    }
  });
  it("requires a nonnegative exact integer lease", () => {
    for (const lease_generation of [undefined, null, "3", -1, 1.5, Number.MAX_SAFE_INTEGER + 1])
      expect(parseSavedSession(JSON.stringify({ ...selection, lease_generation }))).toBeNull();
  });
});

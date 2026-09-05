// The room band model (interface plan I4): renders with ambient absent,
// reads every ambient key defensively, pairs exits with their nav keys.

import { describe, expect, it } from "vitest";

import { exitKeys } from "./keys";
import type { RoomView } from "./proto";
import { clockText, roomBand } from "./roomband";

const base: RoomView = {
  name: "The Hall",
  id: "#2",
  desc: [],
  contents: [],
  exits: [
    { name: "North", aliases: ["n"] },
    { name: "quai", aliases: [] },
  ],
  zone: { id: "#3", name: "Downtown" },
};

describe("roomBand", () => {
  it("renders with ambient absent", () => {
    const band = roomBand(base, exitKeys(base));
    expect(band).not.toBeNull();
    expect(band?.name).toBe("The Hall");
    expect(band?.zone).toBe("Downtown");
    expect(band?.weather).toBeNull();
    expect(band?.clock).toBeNull();
    expect(band?.scene).toBeNull();
    expect(band?.exits).toEqual([
      { name: "North", key: "n" },
      { name: "quai", key: null },
    ]);
    expect(roomBand(null, new Map())).toBeNull();
  });

  it("reads weather, the IC clock and the scene glance", () => {
    const view: RoomView = {
      ...base,
      ambient: {
        weather: { state: "rain", icon: "rain", temp: 9, line: "Overhead: rain." },
        ic_time: { unix: 1, hour: 21, minute: 5, season: "autumn", time_of_day: "night" },
        scene: { id: 4, title: "Rain on the Quai", status: "active", round: 2, turn: 1 },
      },
    };
    const band = roomBand(view, new Map());
    expect(band?.weather).toEqual({ state: "rain", icon: "rain", temp: 9, line: "Overhead: rain." });
    expect(band?.clock).toEqual({ hour: 21, minute: 5, season: "autumn", timeOfDay: "night" });
    expect(clockText(band!.clock!)).toBe("21:05");
    expect(band?.scene).toEqual({ title: "Rain on the Quai", status: "active", round: 2, turn: 1 });
  });

  it("ignores odd ambient shapes", () => {
    const view: RoomView = {
      ...base,
      ambient: { weather: "rain", ic_time: { hour: "nine" }, scene: null, other: [1] },
    };
    const band = roomBand(view, new Map());
    expect(band?.weather).toBeNull();
    expect(band?.clock).toBeNull();
    expect(band?.scene).toBeNull();
  });
});

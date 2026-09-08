// The room band model (interface plan I4): renders with ambient absent,
// reads every ambient key defensively, pairs exits with their nav keys.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

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

  it("carries the module's prose beside the bare state, so the chip can show it", () => {
    // I4-C20: the chip shows `line ?? state` — the module's prose when it
    // sent any, the bare state otherwise. I3's interim `Weather:` scrollback
    // line is retired (render.test.ts), so the band IS where the fact shows;
    // hiding the prose in a `title` tooltip is what kept that line alive.
    const prose = roomBand(
      { ...base, ambient: { weather: { state: "rain", icon: "rain", line: "Overhead: rain." } } },
      new Map(),
    );
    expect(prose?.weather?.line).toBe("Overhead: rain.");

    // A module that sends only a state gets the state — never an empty chip.
    const bare = roomBand({ ...base, ambient: { weather: { state: "rain" } } }, new Map());
    expect(bare?.weather).toEqual({ state: "rain", icon: "rain", temp: null, line: null });

    // The fallback itself lives in the component, which has no harness on
    // this client (`environment: "node"`), so it is pinned as source.
    const src = readFileSync(resolve(process.cwd(), "src/components/RoomBand.svelte"), "utf8");
    expect(src).toContain("{band.weather.line ?? band.weather.state}");
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

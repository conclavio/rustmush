// @vitest-environment jsdom
//
// The two strips around the scene, mounted (doc 43 cut TS.19): the room band
// and the vitals strip. They replace the source probes `lib/roomband.test.ts`
// and `stores/unit.test.ts` carried for the halves a node test cannot render.

import { render, screen } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";

import RoomBand from "./RoomBand.svelte";
import VitalsStrip from "./VitalsStrip.svelte";
import type { GameApp } from "$lib/game.svelte";
import type { RoomView } from "$lib/proto";

const HALL: RoomView = {
  name: "The Hall",
  id: "#2",
  desc: [],
  contents: [],
  exits: [],
  zone: null,
};

function mountBand(roomView: RoomView): void {
  const app = {
    roomView,
    screens: { exits: new Map() },
    presence: { prune: vi.fn(), thinking: [], composing: [] },
  } as unknown as GameApp;
  render(RoomBand, { props: { app, onCmd: vi.fn() } });
}

describe("the room band's weather chip (I4-C20)", () => {
  it("shows the module's prose when it sent any", () => {
    mountBand({
      ...HALL,
      ambient: { weather: { state: "rain", icon: "rain", line: "Overhead: rain." } },
    });
    expect(screen.getByText(/Overhead: rain\./)).toBeTruthy();
  });

  it("falls back to the bare state, never an empty chip", () => {
    mountBand({ ...HALL, ambient: { weather: { state: "rain" } } });
    const chip = screen.getByTitle("rain");
    expect(chip.textContent?.trim()).toContain("rain");
  });
});

describe("the vitals strip", () => {
  it("names its subject visibly, not only as a tooltip", () => {
    // One feed can follow a viewer from subject to subject; a strip that
    // repoints without saying so reads as the old subject's gauges.
    const app = {
      vitals: {
        list: [
          {
            feed: "wod.vitals",
            frame: { subject: { id: "#9", name: "Mara" }, revision: 1, vitals: [] },
          },
        ],
      },
    } as unknown as GameApp;
    render(VitalsStrip, { props: { app } });
    const name = screen.getByText("Mara");
    expect(name.closest("[title]")?.getAttribute("title")).toBe("Mara");
    expect(name.hasAttribute("title")).toBe(false);
  });
});

// @vitest-environment jsdom

import { fireEvent, render, screen } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";

import TacsimPortModals from "./TacsimPortModals.svelte";
import type { GameApp } from "$lib/game.svelte";
import { GUIDED_TOUR_KEYS, QuestsStore } from "$lib/stores/quests.svelte";

function makeApp(
  quests: QuestsStore,
  activeModal: "coop" | "rig" | "sailing" | "trade",
  stowage = {
    cargoKg: 0,
    cargoMaxKg: 100_000,
    cargoL: 0,
    cargoMaxL: 120_000,
    propellantKg: 12_000,
    propellantMaxKg: 80_000,
    provisionsCrewDays: 0,
    provisionsMaxCrewDays: 120,
    cargoLines: [] as Array<{ good: string; label: string; qty: number; unit: string; massKg: number }>,
  },
): GameApp {
  return {
    quests,
    tacsim: {
      activeModal,
      coopSuggestedHull: "Starling",
      activeHullRef: "#99",
      selectedDestination: "stickney",
      selectedQtyPreset: 1,
      goodsById: new Map(),
      marketHistoryLines: [],
      modalLog: [],
      focusedNpc: null,
      detailModal: null,
      ambient: { ownHullName: "Starling", hullName: "Starling", simSlots: [] },
      trade: {
        venueLabel: "Ceres",
        creditsText: "5,000.00cr",
        creditsMinor: 500_000,
        stowage,
        asks: [],
        bids: [],
        hasMore: false,
        nextAfterId: null,
        simulator: null,
      },
      closeModal: vi.fn(),
      requestPortTrade: vi.fn(),
    },
  } as unknown as GameApp;
}

describe("TacsimPortModals Dray-P Vehicle Bay and Co-op Refit quest guidance", () => {
  it("renders live empty-hold readiness inside Co-op Desk for Step 5 refit", () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: GUIDED_TOUR_KEYS[4],
          title: "Refit the Dray for Surface Mining",
          checkpoints: [
            {
              id: "refit_dray_p",
              title: "Refit Dray-P",
              instructions: "Run +coop/refit prospector.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });

    const blocked = render(TacsimPortModals, {
      props: {
        app: makeApp(quests, "coop", {
          cargoKg: 2000,
          cargoMaxKg: 100_000,
          cargoL: 1000,
          cargoMaxL: 120_000,
          propellantKg: 12_000,
          propellantMaxKg: 80_000,
          provisionsCrewDays: 4,
          provisionsMaxCrewDays: 120,
          cargoLines: [{ good: "tacsim:metals", label: "Metals", qty: 2, unit: "u", massKg: 2000 }],
        }),
        onCmd: vi.fn(),
      },
    });
    const blockedStrip = screen.getByLabelText("refit hold readiness");
    expect(blockedStrip.textContent).toContain("[CLEAR HOLD FIRST]");
    expect(blockedStrip.textContent).toContain("Propellant 12 t OK");
    blocked.unmount();

    render(TacsimPortModals, {
      props: { app: makeApp(quests, "coop"), onCmd: vi.fn() },
    });
    const readyStrip = screen.getByLabelText("refit hold readiness");
    expect(readyStrip.textContent).toContain("[READY TO REFIT]");
    expect(readyStrip.textContent).toContain("✓ Cargo empty");
    expect(readyStrip.textContent).toContain("✓ Stores empty");
  });

  it("renders Step 6 prospecting checklist and 1-click rig/prep commands inside Dray-P Vehicle Bay modal", async () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: GUIDED_TOUR_KEYS[5],
          title: "Scan, Sound, and Mine a Landing Site",
          checkpoints: [
            { id: "scan_landing_site", title: "Surface scan", instructions: "Run +tv/1 scan go.", checked: false, live_ready: false },
            { id: "sound_mineral_pocket", title: "Sound pocket", instructions: "Run +tv/1 sound go.", checked: false, live_ready: false },
            { id: "mine_carbon_ore", title: "Mine 1t carbon_ore", instructions: "Run +tv/1 mine go.", checked: false, live_ready: false },
          ],
        },
      ],
    });
    const onCmd = vi.fn();
    render(TacsimPortModals, { props: { app: makeApp(quests, "rig"), onCmd } });

    const banner = screen.getByLabelText("active quest rig bay guidance");
    expect(banner.textContent).toContain("Scan, Sound, and Mine a Landing Site");
    expect(banner.textContent).toContain("carbon_ore");

    await fireEvent.click(screen.getByRole("button", { name: /Scan Site \(\+tv\/1 scan go\)/i }));
    expect(onCmd).toHaveBeenCalledWith("+tv/1 scan go");

    await fireEvent.click(screen.getByRole("button", { name: /License \(\+license\)/i }));
    expect(onCmd).toHaveBeenCalledWith("+license");

    await fireEvent.click(screen.getByRole("button", { name: /Ground Chart \(\+chart\/ground\)/i }));
    expect(onCmd).toHaveBeenCalledWith("+chart/ground");
  });

  it("renders Sailing Plan pre-flight readiness strip and switches to Exchange when missing fuel, stores, or quest cargo", async () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: GUIDED_TOUR_KEYS[2],
          title: "Metals Down the Corridor",
          checkpoints: [
            {
              id: "deliver_metals",
              title: "Deliver 5t metals to Stickney Yards",
              instructions: "Check +ship/plan stickney and sell at Stickney Yards.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });

    const blockedApp = makeApp(quests, "sailing", {
      cargoKg: 2000,
      cargoMaxKg: 100_000,
      cargoL: 1000,
      cargoMaxL: 120_000,
      propellantKg: 0,
      propellantMaxKg: 80_000,
      provisionsCrewDays: 0,
      provisionsMaxCrewDays: 120,
      cargoLines: [{ good: "tacsim:metals", label: "Metals", qty: 2, unit: "u", massKg: 2000 }],
    });
    const blocked = render(TacsimPortModals, { props: { app: blockedApp, onCmd: vi.fn() } });
    const blockedStrip = screen.getByLabelText("sailing pre-flight readiness");
    expect(blockedStrip.textContent).toContain("[CHECK BEFORE UNDOCK]");
    expect(blockedStrip.textContent).toContain("✗ Propellant 0 t");
    expect(blockedStrip.textContent).toContain("✗ Stores 0 d");
    expect(blockedStrip.textContent).toContain("✗ metals 2/5");
    await fireEvent.click(screen.getByRole("button", { name: /Open Exchange/i }));
    expect(blockedApp.tacsim.activeModal).toBe("trade");
    blocked.unmount();

    const readyApp = makeApp(quests, "sailing", {
      cargoKg: 5000,
      cargoMaxKg: 100_000,
      cargoL: 2500,
      cargoMaxL: 120_000,
      propellantKg: 14_000,
      propellantMaxKg: 80_000,
      provisionsCrewDays: 10,
      provisionsMaxCrewDays: 120,
      cargoLines: [{ good: "tacsim:metals", label: "Metals", qty: 5, unit: "u", massKg: 5000 }],
    });
    render(TacsimPortModals, { props: { app: readyApp, onCmd: vi.fn() } });
    const readyStrip = screen.getByLabelText("sailing pre-flight readiness");
    expect(readyStrip.textContent).toContain("[PRE-FLIGHT OK]");
    expect(readyStrip.textContent).toContain("✓ Propellant 14 t");
    expect(readyStrip.textContent).toContain("✓ Stores 10 d");
    expect(readyStrip.textContent).toContain("✓ metals 5/5");
  });
});

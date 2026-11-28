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

  it("auto-selects between-steps Next Guided Offer port in Sailing Plan when off-station and hides once at that port", async () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      completed: [
        { key: GUIDED_TOUR_KEYS[0], title: "S1" },
        { key: GUIDED_TOUR_KEYS[1], title: "S2" },
        { key: GUIDED_TOUR_KEYS[2], title: "S3" },
      ],
      active: [],
      available: [],
    });

    // At Ceres between Steps 3 & 4 -> auto-selects Stickney Yards, renders banner + pre-flight check
    const offStationApp = makeApp(quests, "sailing", {
      cargoKg: 0,
      cargoMaxKg: 100_000,
      cargoL: 0,
      cargoMaxL: 120_000,
      propellantKg: 12_000,
      propellantMaxKg: 80_000,
      provisionsCrewDays: 8,
      provisionsMaxCrewDays: 120,
      cargoLines: [],
    });
    offStationApp.tacsim.selectedDestination = "ceres";
    const onCmd = vi.fn();
    const first = render(TacsimPortModals, { props: { app: offStationApp, onCmd } });

    const banner = await screen.findByRole("status", { name: "active quest destination" });
    expect(banner.textContent).toContain("DESTINATION: STICKNEY YARDS");
    expect(banner.textContent).toContain("Next Guided (4/7): Return Load");
    expect(offStationApp.tacsim.selectedDestination).toBe("stickney");
    expect(screen.getByLabelText("sailing pre-flight readiness").textContent).toContain("[PRE-FLIGHT OK]");
    await fireEvent.click(screen.getByRole("button", { name: /File Sailing Plan/i }));
    expect(onCmd).toHaveBeenCalledWith("+ship/plan stickney");
    first.unmount();

    // Already at Stickney Yards -> clears off-station Next Guided banner
    const atPortApp = makeApp(quests, "sailing");
    (atPortApp.tacsim.trade as unknown as { venueLabel: string }).venueLabel = "Stickney Yards";
    render(TacsimPortModals, { props: { app: atPortApp, onCmd: vi.fn() } });
    expect(screen.queryByRole("status", { name: "active quest destination" })).toBeNull();
  });

  it("shows direct Accept Now and Complete Now quest buttons in NPC Talk modal with name-fallback matching", async () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      room_markers: [
        { npc_id: "#18", npc_name: "Maren Halvorsen", marker: "available", quest_title: "First Papers at the Registry" },
      ],
      available: [
        {
          key: GUIDED_TOUR_KEYS[0],
          title: "First Papers at the Registry",
          giver_id: "#18",
          giver_name: "Maren Halvorsen",
          turn_in_id: "#19",
          turn_in_name: "Ilse Oduya-Brandt",
          accept_cmd: "+quest/accept #18=tok1",
          checkpoints: [],
        },
      ],
      active: [],
      completed: [],
    });

    const offerApp = makeApp(quests, "trade");
    (offerApp.tacsim as unknown as { activeModal: string; focusedNpc: { id: string; name: string; bio: string } }).activeModal = "npc";
    (offerApp.tacsim as unknown as { focusedNpc: { id: string; name: string; bio: string } }).focusedNpc = {
      id: "pack-maren",
      name: "Maren Halvorsen",
      bio: "Port Watch clerk",
    };
    const onCmd1 = vi.fn();
    const first = render(TacsimPortModals, { props: { app: offerApp, onCmd: onCmd1 } });
    expect(screen.getByRole("button", { name: /\[!\] Open Quest Dialog: First Papers at the Registry/i })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: /\[!\] Accept Now/i }));
    expect(onCmd1).toHaveBeenCalledWith("+quest/accept #18=tok1");
    expect(offerApp.tacsim.closeModal).toHaveBeenCalled();
    first.unmount();

    // Ready-to-finish active quest on Ilse -> shows [?] Turn In Quest + [?] Complete Now
    quests.applyFrame({
      schema: 1,
      room_markers: [
        { npc_id: "#19", npc_name: "Ilse Oduya-Brandt", marker: "turn_in", quest_title: "First Papers at the Registry" },
      ],
      active: [
        {
          key: GUIDED_TOUR_KEYS[0],
          title: "First Papers at the Registry",
          giver_id: "#18",
          giver_name: "Maren Halvorsen",
          turn_in_id: "#19",
          turn_in_name: "Ilse Oduya-Brandt",
          ready_to_finish: true,
          can_finish_now: true,
          finish_cmd: "+quest/finish #19=tok2",
          checkpoints: [],
        },
      ],
      available: [],
      completed: [],
    });
    const finishApp = makeApp(quests, "trade");
    (finishApp.tacsim as unknown as { activeModal: string; focusedNpc: { id: string; name: string; bio: string } }).activeModal = "npc";
    (finishApp.tacsim as unknown as { focusedNpc: { id: string; name: string; bio: string } }).focusedNpc = {
      id: "pack-ilse",
      name: "Ilse Oduya-Brandt",
      bio: "Co-op Registrar",
    };
    const onCmd2 = vi.fn();
    render(TacsimPortModals, { props: { app: finishApp, onCmd: onCmd2 } });
    expect(screen.getByRole("button", { name: /\[?\] Turn In Quest: First Papers at the Registry/i })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: /\[?\] Complete Now/i }));
    expect(onCmd2).toHaveBeenCalledWith("+quest/finish #19=tok2");
  });

  it("advances Step 2 Exchange guidance past loaded provisions to propellant and narrows loaded Step 3 metals to SELL bids", async () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: GUIDED_TOUR_KEYS[1],
          title: "Stock the Locker and Tanks",
          checkpoints: [
            {
              id: "buy_provisions",
              title: "Buy at least 10 provisions at Ceres Co-op Chandlery",
              instructions: "Buy 10 provisions.",
              checked: false,
              live_ready: false,
            },
            {
              id: "buy_propellant",
              title: "Buy at least 5 tonnes of propellant at Ceres Exchange Hall",
              instructions: "Buy 5 propellant.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });

    const step2App = makeApp(quests, "trade", {
      cargoKg: 0,
      cargoMaxKg: 100_000,
      cargoL: 0,
      cargoMaxL: 120_000,
      propellantKg: 0,
      propellantMaxKg: 80_000,
      provisionsCrewDays: 10,
      provisionsMaxCrewDays: 120,
      cargoLines: [],
    });
    (step2App.tacsim.trade as unknown as { asks: unknown[] }).asks = [
      {
        listingId: 11,
        goodCode: "tacsim:provisions",
        itemName: "Provisions",
        vendorName: "Chandlery",
        quantity: 20,
        unit: "d",
        unitPriceText: "10 cr",
      },
      {
        listingId: 12,
        goodCode: "tacsim:propellant",
        itemName: "Propellant",
        vendorName: "Exchange",
        quantity: 40,
        unit: "t",
        unitPriceText: "18 cr",
      },
    ];
    const first = render(TacsimPortModals, { props: { app: step2App, onCmd: vi.fn() } });
    const banner2 = screen.getByRole("status", { name: "active quest trade guidance" });
    expect(banner2.textContent).toContain("BUY");
    expect(banner2.textContent).toContain("5 × PROPELLANT");
    expect(banner2.textContent).toContain("10 PROVISIONS");
    await fireEvent.click(screen.getByRole("button", { name: /Set Quest Qty \(5\)/i }));
    expect(step2App.tacsim.selectedQtyPreset).toBe(5);
    first.unmount();

    // Step 3 with 5t metals already loaded -> banner narrows from BUY/SELL to SELL and highlights Standing Bid only
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: GUIDED_TOUR_KEYS[2],
          title: "Metals Down the Corridor",
          checkpoints: [
            {
              id: "deliver_metals",
              title: "Buy 5 tonnes of metals at Ceres and deliver them to Stickney Yards",
              instructions: "Buy 5 metals and sell with +market/sell at Stickney.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });
    const step3App = makeApp(quests, "trade", {
      cargoKg: 5000,
      cargoMaxKg: 100_000,
      cargoL: 2500,
      cargoMaxL: 120_000,
      propellantKg: 10_000,
      propellantMaxKg: 80_000,
      provisionsCrewDays: 10,
      provisionsMaxCrewDays: 120,
      cargoLines: [{ good: "tacsim:metals", label: "Metals", qty: 5, unit: "u", massKg: 5000 }],
    });
    (step3App.tacsim.trade as unknown as { asks: unknown[]; bids: unknown[] }).asks = [
      {
        listingId: 21,
        goodCode: "tacsim:metals",
        itemName: "Metals",
        vendorName: "Ask Desk",
        quantity: 10,
        unit: "t",
        unitPriceText: "100 cr",
      },
    ];
    (step3App.tacsim.trade as unknown as { bids: unknown[] }).bids = [
      {
        listingId: 22,
        goodCode: "tacsim:metals",
        itemName: "Metals",
        vendorName: "Kasei Receiving",
        quantity: 20,
        unit: "t",
        unitPriceText: "145 cr",
      },
    ];
    const { container } = render(TacsimPortModals, { props: { app: step3App, onCmd: vi.fn() } });
    const banner3 = screen.getByRole("status", { name: "active quest trade guidance" });
    expect(banner3.textContent).toContain("SELL");
    expect(banner3.textContent).not.toContain("BUY/SELL");
    expect(banner3.textContent).toContain("5 × METALS");
    const matchedRows = container.querySelectorAll(".lrow.quest-match");
    expect(matchedRows).toHaveLength(1);
    expect(matchedRows[0]?.textContent).toContain("Kasei Receiving");
  });

  it("auto-selects and highlights the tracked quest commodity in 24-Week History", async () => {
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
              title: "Buy 5 tonnes of metals at Ceres and deliver them to Stickney Yards",
              instructions: "Buy 5 metals and sell with +market/sell at Stickney.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });

    const app = makeApp(quests, "trade");
    const onCmd = vi.fn();
    const first = render(TacsimPortModals, { props: { app, onCmd } });

    await fireEvent.click(screen.getByRole("button", { name: "24-Week History" }));
    expect(onCmd).toHaveBeenCalledWith("+market/history tacsim:metals");
    const banner = screen.getByRole("status", { name: "active quest history guidance" });
    expect(banner.textContent).toContain("24-WEEK TREND: METALS");
    expect(screen.getByRole("button", { name: /\[Q\] Refined Metals/i })).toBeTruthy();
    first.unmount();

    // Also verify Step 7 mined carbon_ore uses exact tacsim:carbon_ore history command
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: GUIDED_TOUR_KEYS[6],
          title: "Deliver Your Mined Ore to Market",
          checkpoints: [
            {
              id: "sell_carbon_ore",
              title: "Sell 1t carbon_ore at Stickney",
              instructions: "Sell carbon_ore with +market/sell.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });
    const app7 = makeApp(quests, "trade");
    const onCmd7 = vi.fn();
    render(TacsimPortModals, { props: { app: app7, onCmd: onCmd7 } });
    await fireEvent.click(screen.getByRole("button", { name: "24-Week History" }));
    expect(onCmd7).toHaveBeenCalledWith("+market/history tacsim:carbon_ore");
    expect(screen.getByRole("status", { name: "active quest history guidance" }).textContent).toContain(
      "CARBON ORE",
    );
  });

  it("shows corridor haulage and surface survey quest cross-links inside Jobs & Crew board", async () => {
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
              title: "Buy 5 tonnes of metals at Ceres and deliver them to Stickney Yards",
              instructions: "Run +ship/plan stickney and sell at Stickney.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });

    const app3 = makeApp(quests, "trade");
    const onCmd3 = vi.fn();
    const first = render(TacsimPortModals, { props: { app: app3, onCmd: onCmd3 } });
    await fireEvent.click(screen.getByRole("button", { name: "Jobs & Crew" }));
    const banner3 = screen.getByRole("status", { name: "active quest jobs guidance" });
    expect(banner3.textContent).toContain("CORRIDOR HAULAGE -> STICKNEY YARDS");
    expect(screen.getByRole("button", { name: /\[Q\] Haulage/i })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: /Filter Haulage/i }));
    expect(onCmd3).toHaveBeenCalledWith("+market/jobs haul");
    first.unmount();

    // Step 6 surface prospecting highlights Survey Commissions instead
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: GUIDED_TOUR_KEYS[5],
          title: "Scan, Sound, and Mine a Landing Site",
          checkpoints: [
            {
              id: "scan_landing_site",
              title: "Run a surface scan at a Ceres landing site",
              instructions: "Run +tv/1 scan go.",
              checked: false,
              live_ready: false,
            },
            {
              id: "mine_carbon_ore",
              title: "Mine 1t carbon_ore",
              instructions: "Run +tv/1 mine go.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });
    const app6 = makeApp(quests, "trade");
    const onCmd6 = vi.fn();
    render(TacsimPortModals, { props: { app: app6, onCmd: onCmd6 } });
    await fireEvent.click(screen.getByRole("button", { name: "Jobs & Crew" }));
    const banner6 = screen.getByRole("status", { name: "active quest jobs guidance" });
    expect(banner6.textContent).toContain("SURFACE SURVEY COMMISSIONS");
    expect(screen.getByRole("button", { name: /\[Q\] Survey/i })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: /Filter Surveys/i }));
    expect(onCmd6).toHaveBeenCalledWith("+market/jobs survey");
  });

  it("prioritizes ready turn-in over available offers and shows extra quest count inside NPC Talk modal", async () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      room_id: "#15",
      room_name: "Registry & Co-op Desk",
      room_markers: [
        {
          npc_id: "#50",
          npc_name: "Ilse Oduya-Brandt",
          marker: "available",
          symbol: "!",
          quest_key: GUIDED_TOUR_KEYS[1],
          quest_title: "Locker & Tanks",
          category: "guided",
        },
        {
          npc_id: "#50",
          npc_name: "Ilse Oduya-Brandt",
          marker: "turn_in",
          symbol: "?",
          quest_key: GUIDED_TOUR_KEYS[0],
          quest_title: "First Papers at the Registry",
          category: "guided",
        },
      ],
      available: [
        {
          key: GUIDED_TOUR_KEYS[1],
          title: "Locker & Tanks",
          category: "guided",
          giver_id: "#50",
          giver_name: "Ilse Oduya-Brandt",
          accept_cmd: "+quest/accept #50=tok2",
        },
      ],
      active: [
        {
          key: GUIDED_TOUR_KEYS[0],
          title: "First Papers at the Registry",
          category: "guided",
          giver_id: "#42",
          giver_name: "Maren Halvorsen",
          turn_in_id: "#50",
          turn_in_name: "Ilse Oduya-Brandt",
          ready_to_finish: true,
          can_finish_now: true,
          finish_cmd: "+quest/finish #50=tok1",
          checkpoints: [],
        },
      ],
      completed: [],
    });

    const app = makeApp(quests, "trade");
    Object.assign(app.tacsim, {
      activeModal: "npc",
      focusedNpc: {
        id: "#50",
        name: "Ilse Oduya-Brandt",
        bio: "Registry clerk",
        isNpc: true,
      },
    });
    const onCmd = vi.fn();
    render(TacsimPortModals, { props: { app, onCmd } });

    const turnInBtn = screen.getByRole("button", {
      name: /\[\?\] Turn In Quest: First Papers at the Registry \(\+1 more\)/i,
    });
    expect(turnInBtn).toBeTruthy();

    const finishBtn = screen.getByRole("button", { name: /\[\?\] Complete Now/i });
    await fireEvent.click(finishBtn);
    expect(onCmd).toHaveBeenCalledWith("+quest/finish #50=tok1");
  });

  it("sorts quest-matching Port Asks and Standing Bids first in Commodity Exchange", () => {
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
              title: "Buy 5t metals and sell at Stickney",
              instructions: "Buy 5t metals at Exchange and +market/sell at Stickney.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });

    const app = makeApp(quests, "trade");
    app.tacsim.trade.asks = [
      {
        listingId: 10,
        goodCode: "tacsim:silicates",
        itemName: "Silicates",
        vendorName: "Ceres Depot",
        quantity: 20,
        unit: "t",
        unitPriceMinor: 500,
        unitPriceText: "5.00cr",
      },
      {
        listingId: 11,
        goodCode: "tacsim:metals",
        itemName: "Metals",
        vendorName: "Ceres Exchange",
        quantity: 15,
        unit: "t",
        unitPriceMinor: 1500,
        unitPriceText: "15.00cr",
      },
    ] as unknown as typeof app.tacsim.trade.asks;
    app.tacsim.trade.bids = [
      {
        listingId: 20,
        goodCode: "tacsim:water_ice",
        itemName: "Water Ice",
        vendorName: "Stickney Yards",
        quantity: 30,
        unit: "t",
        unitPriceMinor: 800,
        unitPriceText: "8.00cr",
      },
      {
        listingId: 21,
        goodCode: "tacsim:metals",
        itemName: "Metals",
        vendorName: "Kasei Receiving",
        quantity: 12,
        unit: "t",
        unitPriceMinor: 2200,
        unitPriceText: "22.00cr",
      },
    ] as unknown as typeof app.tacsim.trade.bids;

    const { container } = render(TacsimPortModals, { props: { app, onCmd: vi.fn() } });
    const cols = [...container.querySelectorAll("section.tcol")];
    expect(cols).toHaveLength(2);
    const askRows = [...cols[0]!.querySelectorAll(".lrow")];
    const bidRows = [...cols[1]!.querySelectorAll(".lrow")];
    expect(askRows[0]!.classList.contains("quest-match")).toBe(true);
    expect(askRows[0]!.textContent).toContain("[QUEST] Metals");
    expect(bidRows[0]!.classList.contains("quest-match")).toBe(true);
    expect(bidRows[0]!.textContent).toContain("[QUEST] Metals");
  });

  it("sorts the quest destination card first in Sailing Plan, marks current port [HERE], and shows Step 6 ground expedition banner", async () => {
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
              instructions: "File +ship/plan stickney and sell at Stickney Yards.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });

    const app = makeApp(quests, "sailing");
    const first = render(TacsimPortModals, { props: { app, onCmd: vi.fn() } });
    const cards = [...first.container.querySelectorAll(".dest-card")];
    expect(cards.length).toBeGreaterThan(2);
    expect(cards[0]!.classList.contains("quest-match")).toBe(true);
    expect(cards[0]!.textContent).toContain("[QUEST]");
    expect(cards[0]!.textContent).toContain("Stickney Yards");
    const ceresCard = cards.find((c) => c.textContent?.includes("Ceres Freeport"));
    expect(ceresCard?.textContent).toContain("[HERE]");
    first.unmount();

    // Step 6 active surface mining -> shows ground expedition banner with Ground Chart & Freeport Commons shortcuts
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: GUIDED_TOUR_KEYS[5],
          title: "Scan, Sound, and Mine a Landing Site",
          checkpoints: [
            { id: "scan_landing_site", title: "Surface scan", instructions: "Run +tv/1 scan go.", checked: false, live_ready: false },
            { id: "mine_carbon_ore", title: "Mine 1t carbon_ore", instructions: "Run +tv/1 mine go.", checked: false, live_ready: false },
          ],
        },
      ],
    });
    const step6App = makeApp(quests, "sailing");
    step6App.tacsim.selectedDestination = "stickney";
    const onCmd = vi.fn();
    render(TacsimPortModals, { props: { app: step6App, onCmd } });
    const banner = screen.getByRole("status", { name: "active quest surface expedition" });
    expect(banner.textContent).toMatch(/carbon ore/i);
    const step6Cards = [...document.querySelectorAll(".dest-card")];
    expect(step6Cards[0]!.textContent).toContain("[QUEST]");
    expect(step6Cards[0]!.textContent).toContain("Freeport Commons");
    await fireEvent.click(screen.getByRole("button", { name: /Select Freeport Commons/i }));
    expect(step6App.tacsim.selectedDestination).toBe("commons");
    await fireEvent.click(screen.getByRole("button", { name: /Ground Chart \(\+chart\/ground\)/i }));
    expect(onCmd).toHaveBeenCalledWith("+chart/ground");
  });

  it("shows Step 6/7 badge, live carbon ore hold progress, and post-mine Return button in Rig Bay modal", async () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: GUIDED_TOUR_KEYS[5],
          title: "Scan, Sound, and Mine a Landing Site",
          next_room_entry: "room.c07_exchange",
          checkpoints: [
            { id: "scan_landing_site", title: "Surface scan", instructions: "+tv/1 scan go", checked: true, live_ready: true },
            { id: "sound_pocket", title: "Sound pocket", instructions: "+tv/1 sound go", checked: true, live_ready: true },
            { id: "mine_carbon_ore", title: "Mine 1t carbon_ore", instructions: "+tv/1 mine go", checked: true, live_ready: true },
          ],
        },
      ],
    });

    const app = makeApp(quests, "rig");
    app.tacsim.trade.stowage = {
      ...app.tacsim.trade.stowage,
      cargoKg: 2000,
      cargoLines: [{ good: "tacsim:carbon_ore", label: "Carbon Ore", qty: 2, unit: "t", massKg: 2000 }],
    };
    const onCmd = vi.fn();
    render(TacsimPortModals, { props: { app, onCmd } });

    const banner = screen.getByRole("status", { name: "active quest rig bay guidance" });
    expect(banner.textContent).toContain("Step 6/7");
    expect(banner.textContent).toContain("Hold: 2t carbon ore");
    const retBtn = screen.getByRole("button", { name: /Return → Ceres Freeport \(\+ship\/plan ceres\)/i });
    await fireEvent.click(retBtn);
    expect(onCmd).toHaveBeenCalledWith("+ship/plan ceres");
  });

  it("displays Step n/7 guided badges across Port Exchange, Sailing Plan, Co-op Desk, and NPC Conversation modals", () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: GUIDED_TOUR_KEYS[2],
          title: "Metals Down the Corridor",
          giver_id: "#18",
          giver_name: "Ilse Oduya-Brandt",
          turn_in_id: "#40",
          turn_in_name: "Jonas Achterberg-Nkemelu",
          checkpoints: [
            {
              id: "buy_metals",
              title: "Buy 5t metals at Ceres Exchange",
              instructions: "Buy 5t metals and file +ship/plan stickney.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });

    const tradeApp = makeApp(quests, "trade");
    const rTrade = render(TacsimPortModals, { props: { app: tradeApp, onCmd: vi.fn() } });
    expect(screen.getByRole("status", { name: "active quest trade guidance" }).textContent).toContain("Step 3/7");
    rTrade.unmount();

    const sailApp = makeApp(quests, "sailing");
    const rSail = render(TacsimPortModals, { props: { app: sailApp, onCmd: vi.fn() } });
    expect(screen.getByRole("status", { name: "active quest destination" }).textContent).toContain("Step 3/7");
    rSail.unmount();

    // Step 1 Co-op Desk + NPC Conversation modal
    quests.applyFrame({
      schema: 1,
      room_markers: [
        {
          npc_id: "#11",
          npc_name: "Maren Halvorsen",
          marker: "available",
          symbol: "!",
          quest_key: GUIDED_TOUR_KEYS[0],
          quest_title: "First Papers at the Registry",
        },
      ],
      active: [
        {
          key: GUIDED_TOUR_KEYS[0],
          title: "First Papers at the Registry",
          checkpoints: [
            {
              id: "enroll",
              title: "Enroll in Co-op",
              instructions: "Run +coop/enroll <ship name> at Registry.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
      available: [
        {
          key: GUIDED_TOUR_KEYS[0],
          title: "First Papers at the Registry",
          giver_id: "#11",
          giver_name: "Maren Halvorsen",
          accept_cmd: "+quest/accept #11=tok1",
          checkpoints: [],
        },
      ],
    });

    const coopApp = makeApp(quests, "coop");
    const rCoop = render(TacsimPortModals, { props: { app: coopApp, onCmd: vi.fn() } });
    expect(screen.getByRole("status", { name: "active quest coop guidance" }).textContent).toContain("Step 1/7");
    rCoop.unmount();

    const npcApp = makeApp(quests, "coop");
    npcApp.tacsim.activeModal = "npc";
    npcApp.tacsim.focusedNpc = {
      id: "#11",
      name: "Maren Halvorsen",
      bio: "Port Watch clerk",
      isNpc: true,
    };
    render(TacsimPortModals, { props: { app: npcApp, onCmd: vi.fn() } });
    expect(screen.getByRole("button", { name: /Open Quest Dialog.*Step 1\/7/i })).toBeTruthy();
  });
});

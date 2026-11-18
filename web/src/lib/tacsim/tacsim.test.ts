// @vitest-environment jsdom
//
// Unit & component integration tests for Track 4A (Web 3D Station/Ship Spatial
// Engine, `port.trade` Quote-to-Accept state machine, and Svelte 5 Port Modals).

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { describe, expect, it } from "vitest";

import Game from "../../components/Game.svelte";
import { GameApp } from "../game.svelte";
import type { Envelope, OutputEvent, RoomView } from "../proto";
import type { WebSocketLike } from "../ws";
import {
  computeDoorwaysAndStairs,
  decodeTacsimRoomAmbient,
  filterVisibleRoomActors,
  findShipExitPath,
  findStationExitPath,
  parseNeighborhoodPack,
  parseShipUnitJson,
  tryFindContainingRoomWithHysteresis,
} from "./spatial";
import {
  beginOneClickBuy,
  computeMaxFitQuantity,
  decodePortTradeFeed,
  emptyPortTradeSnapshot,
  parseGoodsJson,
  PORT_TRADE_FEED,
  resolvePendingQuoteFromFeed,
  TACSIM_CONTENT_FEED,
} from "./trade";

const REPO_ROOT = resolve(__dirname, "../../../..");

function readRepoContent(relUnderTacsimContent: string): string {
  return readFileSync(
    resolve(REPO_ROOT, "crates/tacsim/content", relUnderTacsimContent),
    "utf-8",
  );
}

class FakeSocket implements WebSocketLike {
  readyState = 0;
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onclose: ((ev: { code: number; wasClean: boolean }) => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;

  constructor(public url: string) {}
  send(data: string): void {
    this.sent.push(data);
  }
  close(code = 1000): void {
    this.readyState = 3;
    this.onclose?.({ code, wasClean: true });
  }
  deliver(event: OutputEvent, seq: number): void {
    const env = { seq, ts: 1756300000000 + seq, ...event } as Envelope;
    this.onmessage?.({ data: JSON.stringify(env) });
  }
}

HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
  this.open = true;
};
HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
  this.open = false;
};

describe("Track 4A spatial pack parsers & BFS navigation", () => {
  it("parses real Ceres Freeport, Dray, and Dray-P packs and resolves multi-hop BFS paths", () => {
    const ceresJson = readRepoContent("atlas/neighborhood.ceres_freeport.json");
    const ceres = parseNeighborhoodPack(ceresJson, "neighborhood.ceres_freeport");
    expect(ceres.rooms.size).toBeGreaterThanOrEqual(12);
    expect(ceres.rooms.has("room.c01_berth_gallery")).toBe(true);
    expect(ceres.rooms.has("room.c03_arrivals")).toBe(true);

    const { horizontalDoors, verticalDoors, interFloorExits } = computeDoorwaysAndStairs(ceres);
    expect(horizontalDoors.length + verticalDoors.length).toBeGreaterThan(5);
    expect(interFloorExits.length).toBeGreaterThan(0);

    // Multi-hop BFS from Berth Gallery to Exchange Hall
    const stationPath = findStationExitPath(
      ceres,
      "room.c01_berth_gallery",
      "room.c07_exchange",
    );
    expect(stationPath.length).toBeGreaterThanOrEqual(2);
    expect(stationPath[stationPath.length - 1]?.targetRoom).toBe("room.c07_exchange");

    // Hysteresis keeps current room across a 0.6 m doorway threshold band
    const arrivals = ceres.rooms.get("room.c03_arrivals")!;
    const elev = ceres.floors.get(arrivals.floorIndex)?.zOffsetM ?? 6;
    const held = tryFindContainingRoomWithHysteresis(
      ceres,
      { x: arrivals.maxX + 0.5 - 160, y: elev, z: (arrivals.minY + arrivals.maxY) * 0.5 - 150 },
      -160,
      -150,
      "room.c03_arrivals",
      0.85,
    );
    expect(held).toBe("room.c03_arrivals");

    // Dray & Dray-P 3-deck ship interior + Vehicle Bay `+tv/bay in`
    const dray = parseShipUnitJson(readRepoContent("units/dray.json"), "tacsim.unit.dray");
    expect(dray.compartments.has("airlock")).toBe(true);
    expect(dray.compartments.has("bridge")).toBe(true);
    expect(dray.compartments.has("hold")).toBe(true);

    const drayP = parseShipUnitJson(readRepoContent("units/dray_p.json"), "tacsim.unit.dray_p");
    expect(drayP.compartments.has("bay")).toBe(true);
    const bayPath = findShipExitPath(drayP, "airlock", "bay");
    expect(bayPath.length).toBeGreaterThanOrEqual(3);
    expect(bayPath[bayPath.length - 1]).toEqual({
      command: "+tv/bay in",
      targetCompartment: "bay",
    });
  });

  it("filters inanimate Things, dormant NPCs, docked hulls, and self from 3D room actors", () => {
    const view: RoomView = {
      id: "#101",
      name: "Visiting Berth Gallery",
      desc: [{ text: "Floodlit gallery." }],
      exits: [],
      contents: [
        { id: "#10", name: "Self Pilot" },
        { id: "#22", name: "Maya Vance" },
        { id: "#44", name: "Ilse Oduya-Brandt" },
        { id: "#77", name: "Dormant Extra" },
        { id: "#88", name: "Departures Board" },
        { id: "#99", name: "Salt Moth" },
      ],
      ambient: {
        players: ["#10", "#22"],
        npcs: ["#44"],
        dormant_npcs: ["#77"],
        things: ["#88"],
        atlas: { pack: "neighborhood.ceres_freeport", entry: "room.c01_berth_gallery" },
        tacsim: {
          mode: "berth_gallery",
          own_hull: "#99",
          own_hull_name: "Salt Moth",
          own_hull_docked_here: true,
        },
      },
    };
    const dec = decodeTacsimRoomAmbient(view);
    const actors = filterVisibleRoomActors(view, dec, "#10", null);
    expect(actors.map((a) => a.id)).toEqual(["#22", "#44"]);
    expect(actors[0]?.isNpc).toBe(false);
    expect(actors[1]?.isNpc).toBe(true);
  });
});

describe("Track 4A port.trade Quote-to-Accept & Svelte 5 Scene UI", () => {
  it("decodes goods.json + port.trade, computes MAX FIT, and guards 1-click Quote-to-Accept", () => {
    const goods = parseGoodsJson(readRepoContent("market/goods.json"));
    expect(goods.size).toBeGreaterThanOrEqual(6);

    const feed1 = decodePortTradeFeed(
      {
        venue: { id: "ceres.exchange", label: "Freeport Commodity Exchange", port: "Ceres" },
        credit: { minor: 50_000, text: "500.00 Cr" },
        asks: [
          {
            id: 41,
            side: "ask",
            good: "volatiles",
            vendor: "Occator Ice",
            qty: 80,
            unit: "t",
            execution_price_minor: 10_000,
            execution_price_text: "100.00 Cr",
          },
        ],
        bids: [
          {
            id: 52,
            side: "bid",
            good: "metals",
            vendor: "Freeport Works",
            qty: 40,
            unit: "t",
            execution_price_minor: 22_000,
            execution_price_text: "220.00 Cr",
          },
        ],
        stowage: {
          cargo_kg: 10_000,
          cargo_max_kg: 100_000,
          cargo_l: 14_000,
          cargo_max_l: 120_000,
          propellant_kg: 40_000,
          propellant_max_kg: 80_000,
          provisions_crew_days: 30,
          provisions_max_crew_days: 120,
        },
      },
      goods,
      emptyPortTradeSnapshot(),
    );

    expect(feed1.asks).toHaveLength(1);
    expect(feed1.bids).toHaveLength(1);
    const ask = feed1.asks[0]!;
    // 500 Cr / 100 Cr per t = 5 t affordable
    expect(computeMaxFitQuantity(ask, goods, feed1.stowage, feed1.creditsMinor)).toBe(5);

    const started = beginOneClickBuy(ask, 4, 1_000)!;
    expect(started.quoteCommand).toBe("+market/quote 41=4");

    // Matching quote at <= maxUnitPriceMinor auto-accepts
    const withGoodQuote = decodePortTradeFeed(
      { quotes: [{ id: 901, listing_id: 41, qty: 4, unit_price_minor: 10_000 }] },
      goods,
      feed1,
    );
    const okRes = resolvePendingQuoteFromFeed(withGoodQuote, started.latch, 1_500);
    expect(okRes.acceptCommand).toBe("+market/accept 901");
    expect(okRes.priceWarning).toBeNull();

    // Quote whose price moved above maxUnitPriceMinor refuses auto-accept and warns
    const withMovedQuote = decodePortTradeFeed(
      { quotes: [{ id: 902, listing_id: 41, qty: 4, unit_price_minor: 13_500 }] },
      goods,
      feed1,
    );
    const warnRes = resolvePendingQuoteFromFeed(withMovedQuote, started.latch, 1_500);
    expect(warnRes.acceptCommand).toBeNull();
    expect(warnRes.priceWarning).toContain("Quote #902 moved");
  });

  it("mounts TacsimSceneDeck & Port Exchange modal inside Game without breaking the single command box", async () => {
    let ws: FakeSocket | null = null;
    const app = new GameApp({
      url: "ws://test/ws",
      webSocketFactory: (url) => (ws = new FakeSocket(url)),
      random: () => 1,
    });
    app.client.connect("tok", "7");
    const socket = ws as unknown as FakeSocket;
    socket.readyState = 1;
    socket.onopen?.();
    socket.deliver(
      {
        t: "hello",
        d: {
          proto: 1,
          server: "test",
          features: [],
          panels: [PORT_TRADE_FEED, TACSIM_CONTENT_FEED, "space.cockpit", "space.holotank"],
        },
      } as OutputEvent,
      0,
    );

    render(Game, { props: { app } });
    await tick();

    // Deliver a Ceres Exchange room view
    socket.deliver(
      {
        t: "room",
        d: {
          id: "#107",
          name: "Exchange Hall",
          desc: [{ text: "Busy commodity floor." }],
          contents: [],
          exits: [],
          ambient: {
            atlas: { pack: "neighborhood.ceres_freeport", entry: "room.c07_exchange", surface: "station" },
          },
        },
      } as OutputEvent,
      1,
    );
    await tick();

    // Port toolbar renders with `3D VIEW` and `Port Exchange` quick action
    const exchBtn = screen.getByRole("button", { name: "Port Exchange" });
    await fireEvent.click(exchBtn);
    await tick();

    expect(app.tacsim.activeModal).toBe("trade");
    // Single `"command input"` invariant still holds even with Port Exchange open!
    expect(screen.getAllByRole("textbox", { name: "command input" })).toHaveLength(1);

    // Narrative speech populates `actorSpeech` for 3D billboards
    socket.deliver(
      {
        t: "narrative",
        d: {
          kind: "say",
          actor: { id: "#44", name: "Margit Søndergaard" },
          text: [{ text: "Concentrate bids are firm today." }],
        },
      } as OutputEvent,
      2,
    );
    expect(app.tacsim.actorSpeech["#44"]?.text).toBe("Concentrate bids are firm today.");

    // Bridge Helm visor handoff
    app.tacsim.closeModal();
    socket.deliver(
      {
        t: "room",
        d: {
          id: "#802",
          name: "Salt Moth - Bridge",
          desc: [{ text: "Two couches before displays." }],
          contents: [],
          exits: [],
          ambient: {
            tacsim: { mode: "interior", unit: "dray", room: "bridge", hull_name: "Salt Moth", docked: false },
          },
        },
      } as OutputEvent,
      3,
    );
    await tick();

    expect(app.tacsim.isAboardShip).toBe(true);
    app.tacsim.toggle3dView(true);
    const helmBtn = screen.getByRole("button", { name: "Take Bridge Helm" });
    await fireEvent.click(helmBtn);
    await tick();

    expect(app.tacsim.seatedAtBridgeHelm).toBe(true);
    const standBtns = screen.getAllByRole("button", { name: "Stand From Helm" });
    expect(standBtns.length).toBeGreaterThanOrEqual(1);
    await fireEvent.click(standBtns[0]!);
    await tick();
    expect(app.tacsim.seatedAtBridgeHelm).toBe(false);
  });
});

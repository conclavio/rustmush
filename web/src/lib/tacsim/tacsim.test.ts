// @vitest-environment jsdom
//
// Unit & component integration tests for Track 4A (Web 3D Station/Ship Spatial
// Engine, `port.trade` Quote-to-Accept state machine, and Svelte 5 Port Modals).

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { describe, expect, it, vi } from "vitest";

import Game from "../../components/Game.svelte";
import QuarryScreen from "../../components/QuarryScreen.svelte";
import SheetPanel from "../../components/SheetPanel.svelte";
import TacsimSceneDeck from "../../components/TacsimSceneDeck.svelte";
import { GameApp } from "../game.svelte";
import type { Envelope, OutputEvent, RoomView } from "../proto";
import {
  PACK_IN_FLIGHT_TIMEOUT_MS,
  PACK_MAX_RETRIES,
  PACK_RETRY_BASE_DELAY_MS,
  TacsimPortStore,
} from "../stores/tacsim.svelte";
import type { WebSocketLike } from "../ws";
import {
  atlasToWorldPos,
  buildFallbackFreeport,
  classifyStationDesk,
  computeDoorwaysAndStairs,
  computeInterFloorStairPlacement,
  computeNextQuestExitBreadcrumb,
  computeRightHandedMoveDelta,
  contextualQuickActions,
  decodeTacsimRoomAmbient,
  filterVisibleRoomActors,
  findShipExitPath,
  findStationExitPath,
  formatShipTraversalCommand,
  formatStationExitCommand,
  getFloorElevationY,
  normalizePortVenueTarget,
  parseNeighborhoodPack,
  parseShipUnitJson,
  resolveDynamicDockTarget,
  RoomBoundNavWalker,
  tryFindContainingCompartment,
  tryFindContainingRoomWithHysteresis,
  worldToAtlas2D,
} from "./spatial";
import {
  beginOneClickBuy,
  computeMaxFitQuantity,
  CORRIDOR_DESTINATIONS,
  decodePortTradeFeed,
  emptyPortTradeSnapshot,
  parseGoodsJson,
  PORT_TRADE_FEED,
  resolvePendingQuoteFromFeed,
  TACSIM_CONTENT_FEED,
} from "./trade";
import {
  decodeSpaceFireFeed,
  formatHeldLockSummary,
  formatMountAtLockEffect,
  formatMountMeta,
  formatMountStateLabel,
} from "./space-fire";
import {
  computeQuestWayfinderPoints,
  formatActorInteractPrompt,
  TacsimStage3D,
} from "./stage3d";

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

    // Right-handed atlasToWorldPos / worldToAtlas2D (`W3D-C2`: East=+X, North=-Z)
    const arrivals = ceres.rooms.get("room.c03_arrivals")!;
    const elev = ceres.floors.get(arrivals.floorIndex)?.zOffsetM ?? 6;
    const probePos = atlasToWorldPos(
      arrivals.maxX + 0.5,
      elev,
      (arrivals.minY + arrivals.maxY) * 0.5,
      -160,
      -150,
    );
    const back = worldToAtlas2D(probePos.x, probePos.z, -160, -150);
    expect(back.x).toBeCloseTo(arrivals.maxX + 0.5, 5);
    expect(back.y).toBeCloseTo((arrivals.minY + arrivals.maxY) * 0.5, 5);
    // North (+ay) maps to -Z, East (+ax) maps to +X
    const northPos = atlasToWorldPos(150, elev, 180, -160, -150);
    const southPos = atlasToWorldPos(150, elev, 120, -160, -150);
    expect(northPos.z).toBeLessThan(southPos.z);

    // Right-handed camera strafing (`W3D-C2`):
    // Facing North (`camYawRad = Math.PI`, Forward = -Z), pressing D (`rightIn = +1`) moves +X (East / screen-right).
    const strafeRightFacingNorth = computeRightHandedMoveDelta(Math.PI, 0, 1, 5, 1);
    expect(strafeRightFacingNorth.dx).toBeCloseTo(5, 5);
    expect(strafeRightFacingNorth.dz).toBeCloseTo(0, 5);
    // Facing South (`camYawRad = 0`, Forward = +Z), pressing D (`rightIn = +1`) moves -X (screen-right).
    const strafeRightFacingSouth = computeRightHandedMoveDelta(0, 0, 1, 5, 1);
    expect(strafeRightFacingSouth.dx).toBeCloseTo(-5, 5);
    expect(strafeRightFacingSouth.dz).toBeCloseTo(0, 5);

    // Hysteresis keeps current room across a 0.5 m doorway threshold band
    const held = tryFindContainingRoomWithHysteresis(
      ceres,
      probePos,
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

    // W3D-H7: vertical hysteresis must not shadow bridge (+1.8) with passage, or passage (-1.6) with hold
    expect(tryFindContainingCompartment(dray, { x: 0, y: 1.8, z: 7.0 }, "passage", 0.55)).toBe("bridge");
    expect(tryFindContainingCompartment(dray, { x: 0, y: -1.6, z: 2.0 }, "hold", 0.55)).toBe("passage");
    expect(tryFindContainingCompartment(dray, { x: 0, y: -5.3, z: 2.0 }, "passage", 0.55)).toBe("hold");

    const drayP = parseShipUnitJson(readRepoContent("units/dray_p.json"), "tacsim.unit.dray_p");
    expect(drayP.compartments.has("bay")).toBe(true);
    const bayPath = findShipExitPath(drayP, "airlock", "bay");
    expect(bayPath.length).toBeGreaterThanOrEqual(3);
    expect(bayPath[bayPath.length - 1]).toEqual({
      command: "+tv/bay in",
      targetCompartment: "bay",
    });
  });

  it("W3D-H3 & W3D-M6: places inter-floor stairs inside both upper and lower footprints and classifies non-Ceres desks", () => {
    const multiFloorPorts = [
      "neighborhood.aitken_highport.json",
      "neighborhood.ceres_freeport.json",
      "neighborhood.stickney_yards.json",
      "neighborhood.voltaire_anchorage.json",
      "neighborhood.ligeia_terminal.json",
    ];
    for (const file of multiFloorPorts) {
      const packId = file.replace(/\.json$/, "");
      const pack = parseNeighborhoodPack(readRepoContent(`atlas/${file}`), packId);
      for (const ex of pack.exits) {
        const rA = pack.rooms.get(ex.fromRoom);
        const rB = pack.rooms.get(ex.toRoom);
        if (!rA || !rB || rA.floorIndex === rB.floorIndex) continue;
        const st = computeInterFloorStairPlacement(pack, ex);
        expect(st).not.toBeNull();
        const upper = pack.rooms.get(st!.upperRoomId)!;
        const lower = pack.rooms.get(st!.lowerRoomId)!;
        expect(upper.floorIndex).toBeGreaterThan(lower.floorIndex);
        // Entire stair cutout must lie strictly inside BOTH upper and lower rooms!
        expect(st!.cutMinAx).toBeGreaterThanOrEqual(upper.minX);
        expect(st!.cutMaxAx).toBeLessThanOrEqual(upper.maxX);
        expect(st!.cutMinAy).toBeGreaterThanOrEqual(upper.minY);
        expect(st!.cutMaxAy).toBeLessThanOrEqual(upper.maxY);
        expect(st!.cutMinAx).toBeGreaterThanOrEqual(lower.minX);
        expect(st!.cutMaxAx).toBeLessThanOrEqual(lower.maxX);
        expect(st!.cutMinAy).toBeGreaterThanOrEqual(lower.minY);
        expect(st!.cutMaxAy).toBeLessThanOrEqual(lower.maxY);
      }
    }

    // W3D-M6: parseNeighborhoodPack preserves manifest.id (not bare "neighborhood"), Ceres registry
    // gets Co-op desk, Aitken registry does NOT, and Stickney/Voltaire/Ligeia desks resolve.
    const ceres = parseNeighborhoodPack(readRepoContent("atlas/neighborhood.ceres_freeport.json"));
    expect(ceres.packId).toBe("neighborhood.ceres_freeport");
    expect(classifyStationDesk(ceres.packId, ceres.rooms.get("room.c05_registry")!)?.deskKey).toBe("ui:open_coop");
    expect(normalizePortVenueTarget("neighborhood")).toBeNull();

    const aitken = parseNeighborhoodPack(readRepoContent("atlas/neighborhood.aitken_highport.json"));
    expect(aitken.packId).toBe("neighborhood.aitken_highport");
    const aitkenReg = classifyStationDesk(aitken.packId, aitken.rooms.get("room.l05_registry")!);
    expect(aitkenReg?.deskKey).toBe("ui:open_sailing");
    expect(aitkenReg?.title).not.toContain("Co-op");

    const stickney = parseNeighborhoodPack(readRepoContent("atlas/neighborhood.stickney_yards.json"));
    expect(stickney.packId).toBe("neighborhood.stickney_yards");
    expect(classifyStationDesk(stickney.packId, stickney.rooms.get("room.p05_harbourmaster")!)?.deskKey).toBe("ui:open_sailing");
    expect(classifyStationDesk(stickney.packId, stickney.rooms.get("room.p09_commissary")!)?.deskKey).toBe("ui:open_trade");

    const voltaire = parseNeighborhoodPack(readRepoContent("atlas/neighborhood.voltaire_anchorage.json"));
    expect(voltaire.packId).toBe("neighborhood.voltaire_anchorage");
    expect(classifyStationDesk(voltaire.packId, voltaire.rooms.get("room.d05_harbourmaster")!)?.deskKey).toBe("ui:open_sailing");
    expect(classifyStationDesk(voltaire.packId, voltaire.rooms.get("room.d09_commissary")!)?.deskKey).toBe("ui:open_trade");

    const ligeia = parseNeighborhoodPack(readRepoContent("atlas/neighborhood.ligeia_terminal.json"));
    expect(ligeia.packId).toBe("neighborhood.ligeia_terminal");
    expect(classifyStationDesk(ligeia.packId, ligeia.rooms.get("room.t11_bunkering")!)?.deskKey).toBe("ui:open_trade");
  });

  it("W3D-H6: maps surface 'ground' to floorIndex 0 (elevation 0m) on real neighborhood.ooc_area.json", () => {
    const oocJson = readFileSync(
      resolve(REPO_ROOT, "crates/mush-atlas/content/neighborhood.ooc_area.json"),
      "utf-8",
    );
    const ooc = parseNeighborhoodPack(oocJson, "neighborhood.ooc_area");
    for (const roomId of ["room.welcome_hall", "room.simulator_lobby", "room.sim_red", "room.sim_blue", "room.bus_stop"]) {
      const r = ooc.rooms.get(roomId);
      expect(r, `expected ${roomId}`).toBeDefined();
      expect(r!.floorIndex).toBe(0);
      expect(getFloorElevationY(ooc, r!.floorIndex)).toBe(0);
    }
  });

  it("W3D-H5 & W3D-L4: decodes live RoomView.ambient.occupants.{npcs,dormant,players,things} and tac.hull_id", () => {
    // Matches `crates/mush-npc/src/module.rs:4570-4589` and `crates/tacsim/src/port/ship.rs:1455-1507`
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
        { id: "#88", name: "Mooring Bollard" },
        { id: "#99", name: "Salt Moth" },
      ],
      ambient: {
        occupants: {
          players: ["#10", "#22"],
          npcs: ["#44"],
          dormant: ["#77"],
          things: ["#88", "#99"],
        },
        atlas: { pack: "neighborhood.ceres_freeport", entry: "room.c01_berth_gallery" },
        tacsim: {
          mode: "berth_gallery",
          port_entry: "ceres_port",
          docked_count: 1,
          own_hull: "#99",
          own_hull_name: "Salt Moth",
          own_hull_docked_here: true,
        },
      },
    };
    const dec = decodeTacsimRoomAmbient(view);
    expect(dec.hasOccupantsManifest).toBe(true);
    expect(dec.dormantNpcIds.has("#77")).toBe(true);
    expect(dec.thingIds.has("#88")).toBe(true);
    const actors = filterVisibleRoomActors(view, dec, "#10", null);
    // Other player #22 MUST render in Berth Gallery; dormant #77 & Thing #88/#99 must NOT!
    expect(actors.map((a) => a.id)).toEqual(["#22", "#44"]);
    expect(actors[0]?.isNpc).toBe(false);
    expect(actors[1]?.isNpc).toBe(true);

    // Ship interior ambient carries `hull_id` (`W3D-L4`)
    const shipDec = decodeTacsimRoomAmbient({
      id: "#802",
      name: "Salt Moth - Bridge",
      desc: [],
      exits: [],
      contents: [],
      ambient: {
        tacsim: {
          mode: "interior",
          hull_id: "#99",
          hull_name: "Salt Moth",
          unit: "tacsim.unit.dray",
          room: "bridge",
          docked: false,
        },
      },
    });
    expect(shipDec.hullId).toBe("#99");
  });
});

describe("Corridor destinations (48e L8, clients:G7)", () => {
  it("offers exactly the lane's venue words", () => {
    const lane = JSON.parse(readRepoContent("lane/ceres_phobos.json")) as {
      entries: Record<string, { kind: string; def: Record<string, any> }>;
    };
    const venues = Object.values(lane.entries)
      .filter((e) => e.kind === "port")
      .map((e) => e.def["venue"] as string)
      .sort();
    expect(CORRIDOR_DESTINATIONS.map((d) => d.id).sort()).toEqual(venues);
  });
});

describe("Track 4A port.trade Quote-to-Accept & Svelte 5 Scene UI", () => {
  it("C-19 & W3D-H4 & H-40: decodes LIVE client_sync.rs port.trade schema (berth, open_quotes, stowage.items, next_after)", () => {
    const goods = parseGoodsJson(readRepoContent("market/goods.json"));
    expect(goods.size).toBeGreaterThanOrEqual(6);

    // Exact live JSON shape emitted by `crates/tacsim/src/client_sync.rs:1206-1347`
    // and `crates/tacsim/src/port/ship.rs:1639-1773`.
    const feed1 = decodePortTradeFeed(
      {
        schema: { id: "port.trade", version: 1 },
        enabled: true,
        venue: "ceres",
        port_name: "Ceres Freeport",
        aboard_docked_port: null,
        balance_minor: 50_000,
        balance_text: "500.00cr",
        listings: [
          {
            id: 41,
            side: "ask",
            good: "volatiles",
            good_label: "water ice",
            vendor_name: "Occator Ice",
            is_personal: false,
            funded: true,
            unit_price_minor: 10_000,
            price_text: "100.00cr",
            available_qty: 80,
          },
          {
            id: 52,
            side: "bid",
            good: "metals",
            good_label: "metal concentrate",
            vendor_name: "Freeport Works",
            is_personal: false,
            funded: true,
            unit_price_minor: 22_000,
            price_text: "220.00cr",
            available_qty: 40,
          },
        ],
        next_after: 52,
        open_quotes: [],
        ship: {
          hull_id: "#99",
          hull_name: "Salt Moth",
          mark: "CF-DR-01",
          unit: "tacsim.unit.dray",
          class: "Dray",
          docked: true,
          port: "ceres_port",
          standing: "charter",
          sales: 2,
          buyout_minor: 150_000,
          buyout_text: "1500.00cr",
          stowage: {
            cargo_kg: 10_000,
            cargo_limit_kg: 100_000,
            cargo_l: 14_000,
            cargo_limit_l: 120_000,
            tank_kg: 40_000,
            tank_limit_kg: 80_000,
            stores_days: 30,
            stores_limit_days: 120,
            items: [{ good: "volatiles", qty: 10 }],
          },
        },
        berth: {
          port_entry: "ceres_port",
          docked_count: 2,
          docked_hulls: [
            { id: "#99", name: "Salt Moth", mark: "CF-DR-01", unit: "tacsim.unit.dray" },
            { id: "#104", name: "Copper Lantern", mark: "CF-DR-02", unit: "tacsim.unit.dray" },
          ],
          own_hull: "#99",
          own_hull_name: "Salt Moth",
          own_hull_docked_here: true,
        },
        simulator: {
          room: "lobby",
          in_pod: false,
          match_state: "staging",
          ai_fill: 4,
          my_team: "red",
          my_unit: "tacsim.unit.line_tank",
          slots: [
            { team: "red", index: 1, pilot: "#10", pilot_name: "Pilot", unit: "tacsim.unit.line_tank", seated: true },
          ],
        },
      },
      goods,
      emptyPortTradeSnapshot(),
    );

    expect(feed1.venueId).toBe("ceres");
    expect(feed1.portName).toBe("Ceres Freeport");
    expect(feed1.creditsMinor).toBe(50_000);
    expect(feed1.creditsText).toBe("500.00cr");
    expect(feed1.hasMore).toBe(true);
    expect(feed1.nextAfterId).toBe(52);
    expect(feed1.hullId).toBe("#99");
    expect(feed1.hullName).toBe("Salt Moth");
    expect(feed1.stowage.cargoMaxKg).toBe(100_000);
    expect(feed1.stowage.propellantMaxKg).toBe(80_000);
    expect(feed1.stowage.provisionsMaxCrewDays).toBe(120);
    expect(feed1.stowage.cargoLines).toHaveLength(1);
    expect(feed1.stowage.cargoLines[0]?.good).toBe("volatiles");
    expect(feed1.stowage.cargoLines[0]?.qty).toBe(10);
    expect(feed1.berth?.dockedHulls).toHaveLength(2);
    expect(feed1.berth?.dockedHulls[1]?.name).toBe("Copper Lantern");
    expect(feed1.simulator?.slots).toHaveLength(1);

    expect(feed1.asks).toHaveLength(1);
    expect(feed1.bids).toHaveLength(1);
    const ask = feed1.asks[0]!;
    expect(ask.itemName).toBe("water ice");
    expect(ask.vendorName).toBe("Occator Ice");
    expect(ask.quantity).toBe(80);
    // 500 Cr / 100 Cr per t = 5 t affordable
    expect(computeMaxFitQuantity(ask, goods, feed1.stowage, feed1.creditsMinor)).toBe(5);

    const started = beginOneClickBuy(ask, 4, 1_000)!;
    expect(started.quoteCommand).toBe("+market/quote 41=4");

    // Live `open_quotes` key with `quote_id` & `quantity` auto-accepts when <= maxUnitPriceMinor
    const withGoodQuote = decodePortTradeFeed(
      {
        open_quotes: [
          {
            quote_id: 901,
            listing_id: 41,
            good: "volatiles",
            market: "ceres",
            quantity: 4,
            unit_price_minor: 10_000,
            total_minor: 40_000,
          },
        ],
      },
      goods,
      feed1,
    );
    const okRes = resolvePendingQuoteFromFeed(withGoodQuote, started.latch, 1_500);
    expect(okRes.acceptCommand).toBe("+market/accept 901");
    expect(okRes.priceWarning).toBeNull();

    // Quote whose price moved above maxUnitPriceMinor refuses auto-accept and warns
    const withMovedQuote = decodePortTradeFeed(
      {
        open_quotes: [
          { quote_id: 902, listing_id: 41, quantity: 4, unit_price_minor: 13_500, total_minor: 54_000 },
        ],
      },
      goods,
      feed1,
    );
    const warnRes = resolvePendingQuoteFromFeed(withMovedQuote, started.latch, 1_500);
    expect(warnRes.acceptCommand).toBeNull();
    expect(warnRes.priceWarning).toContain("Quote #902 moved");

    // H-40 & 48e L8 (clients:G7): all 13 canonical corridor venue IDs (doc 50 cut OP.1 adds
    // isidis) and dynamic dock target
    expect(CORRIDOR_DESTINATIONS.map((d) => d.id)).toEqual([
      "ceres",
      "stickney",
      "luna",
      "deimos",
      "titan",
      "commons",
      "kerwan",
      "ahuna",
      "juling",
      "occator",
      "rheasilvia",
      "urvara",
      "isidis",
    ]);
    expect(resolveDynamicDockTarget({ cockpitSiteId: "stickney_yards" })).toBe("stickney");
    expect(resolveDynamicDockTarget({ stationPackId: "neighborhood.aitken_highport" })).toBe("luna");
    expect(resolveDynamicDockTarget({ stationPackId: "neighborhood.freeport_commons" })).toBe("commons");
    expect(resolveDynamicDockTarget({ stationPackId: "neighborhood.rheasilvia_commons" })).toBe("rheasilvia");
    expect(resolveDynamicDockTarget({ selectedDestination: "titan" })).toBe("titan");
    const helmActions = contextualQuickActions("bridge", true, true, "stickney");
    expect(helmActions.some((a) => a.action === "+helm/dock stickney")).toBe(true);
  });

  it("decodes a real port.trade frame, computes MAX FIT, and guards 1-click Quote-to-Accept", () => {
    const goods = parseGoodsJson(readRepoContent("market/goods.json"));
    expect(goods.size).toBeGreaterThanOrEqual(6);

    // 48e L8 (clients:G2): a frame the server built, held to the live shape by
    // tacsim's `port_trade_frame_matches_the_clients_shared_fixture`.
    const frame = JSON.parse(
      readFileSync(resolve(REPO_ROOT, "web/src/lib/fixtures/port/port_trade.json"), "utf-8"),
    ) as Record<string, any>;
    const listings = frame.listings as Array<Record<string, any>>;
    const askRow = listings.find((r) => r.side === "ask")!;
    const bidRow = listings.find((r) => r.side === "bid")!;
    const stow = frame.ship.stowage as Record<string, any>;

    const feed1 = decodePortTradeFeed(frame, goods, emptyPortTradeSnapshot());
    expect(feed1.creditsMinor).toBe(frame.balance_minor);
    expect(feed1.creditsText).toBe(frame.balance_text);
    expect(feed1.portName).toBe(frame.port_name);
    expect(feed1.asks.length + feed1.bids.length).toBe(listings.length);
    expect(feed1.bids.length).toBeGreaterThan(0);
    const ask = feed1.asks.find((a) => a.listingId === askRow.id)!;
    expect(ask.quantity).toBe(askRow.available_qty);
    expect(ask.itemName).toBe(askRow.good_label);
    expect(ask.vendorName).toBe(askRow.vendor_name);
    expect(feed1.bids.find((b) => b.listingId === bidRow.id)?.quantity).toBe(bidRow.available_qty);
    expect(feed1.quotes.map((q) => q.quoteId)).toEqual(
      (frame.open_quotes as Array<Record<string, any>>).map((q) => q.quote_id),
    );
    expect(feed1.hullName).toBe(frame.ship.hull_name);
    expect(feed1.hullId).toBe(frame.ship.hull_id);
    expect(feed1.stowage.cargoMaxKg).toBe(Math.max(1, stow.cargo_limit_kg));
    expect(feed1.stowage.cargoMaxL).toBe(Math.max(1, stow.cargo_limit_l));
    expect(feed1.stowage.propellantMaxKg).toBe(Math.max(1, stow.tank_limit_kg));
    expect(feed1.stowage.provisionsMaxCrewDays).toBe(stow.stores_limit_days);
    expect(feed1.stowage.cargoLines.map((l) => [l.good, l.qty])).toEqual(
      (stow.items as Array<Record<string, any>>).map((i) => [i.good, i.qty]),
    );
    expect(feed1.hasMore).toBe(frame.next_after !== null);
    expect(feed1.nextAfterId).toBe(frame.next_after);

    // MAX FIT: the wallet bounds it at floor(balance / price), the listing at its stock.
    const expectFit = Math.max(
      1,
      Math.min(ask.quantity, Math.floor(feed1.creditsMinor / ask.unitPriceMinor)),
    );
    expect(computeMaxFitQuantity(ask, goods, feed1.stowage, feed1.creditsMinor)).toBeLessThanOrEqual(
      expectFit,
    );

    const started = beginOneClickBuy(ask, 4, 1_000, feed1.quotes)!;
    expect(started.quoteCommand).toBe(`+market/quote ${ask.listingId}=4`);

    // Matching quote at <= maxUnitPriceMinor auto-accepts
    const quoteRow = (price: number, id: number) => ({
      open_quotes: [
        { quote_id: id, listing_id: ask.listingId, quantity: 4, unit_price_minor: price, total_minor: price * 4 },
      ],
    });
    const withGoodQuote = decodePortTradeFeed(quoteRow(ask.unitPriceMinor, 901), goods, feed1);
    const okRes = resolvePendingQuoteFromFeed(withGoodQuote, started.latch, 1_500);
    expect(okRes.acceptCommand).toBe("+market/accept 901");
    expect(okRes.priceWarning).toBeNull();

    // Quote whose price moved above maxUnitPriceMinor refuses auto-accept and warns
    const withMovedQuote = decodePortTradeFeed(quoteRow(ask.unitPriceMinor + 3_500, 902), goods, feed1);
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

    // Deliver a Doc 49 CC.9 `space.cockpit` frame while seated at Bridge Helm
    socket.deliver(
      {
        t: "panel",
        d: {
          feed: "space.cockpit",
          kind: "keyframe",
          data: {
            active: true,
            yaw_deg: 20,
            thrust_permille: 500,
            emcon: "ACTIVE",
            fire: {
              mounts: [
                {
                  id: "laser_dorsal",
                  family: "laser",
                  ready_in_ms: 0,
                  state: "ready",
                  mode: "offence",
                  damage: 98,
                },
                {
                  id: "rail_spinal",
                  family: "spinal",
                  ready_in_ms: 2400,
                  state: "cycling",
                  rounds: 31,
                  rounds_max: 32,
                  flight_ms: 2200,
                },
              ],
              lock_id: "raider.7.1",
              lock: {
                id: "raider.7.1",
                label: "Belt Raider",
                range_m: 42_000,
                facet: "top",
              },
            },
          },
        },
      } as OutputEvent,
      4,
    );
    await tick();

    expect(screen.getByText(/Strike Band: Top/)).toBeTruthy();
    expect(screen.getByText("98 dmg")).toBeTruthy();
    expect(screen.getByText("Spinal · 31/32")).toBeTruthy();
    expect(screen.getByText("Cycling 2.4s")).toBeTruthy();

    const laserDefBtn = screen.getByRole("button", { name: "Laser Defense" });
    await fireEvent.click(laserDefBtn);
    expect(socket.sent.some((raw) => raw.includes("+tac/pdc point"))).toBe(true);

    const standBtns = screen.getAllByRole("button", { name: "Stand From Helm" });
    expect(standBtns.length).toBeGreaterThanOrEqual(1);
    await fireEvent.click(standBtns[0]!);
    await tick();
    expect(app.tacsim.seatedAtBridgeHelm).toBe(false);
  });

  it("decodes all four Doc 49 CC.9 space mount families, persistent magazines, and 5-band hull aspect", () => {
    const snap = decodeSpaceFireFeed({
      emcon: "SILENT",
      thrust_permille: 350,
      mining: {
        matched_rock: "b",
        approaching_rock: null,
        active_drones: 2,
        laser: { rock: "b" },
      },
      fire: {
        mounts: [
          { id: "laser_1", family: "laser", state: "ready", ready_in_ms: 0, mode: "offence", damage: 112 },
          { id: "laser_2", family: "laser", state: "ready", ready_in_ms: 0, mode: "defence" },
          { id: "tube_1", family: "tubes", state: "ready", ready_in_ms: 0, rounds: 3, rounds_max: 4 },
          { id: "spinal_1", family: "spinal", state: "too_hot", ready_in_ms: 0, rounds: 15, rounds_max: 16, flight_ms: 1800 },
          { id: "pdc_1", family: "point_defence", state: "ready", ready_in_ms: 0, rounds: 4000 },
        ],
        lock_id: "corsair.2",
        lock: { id: "corsair.2", label: "Red Corsair", range_m: 18_500, facet: "rear" },
      },
    });

    expect(snap.mounts).toHaveLength(5);
    expect(formatMountMeta(snap.mounts[0]!)).toBe("Laser · Offense");
    expect(formatMountAtLockEffect(snap.mounts[0]!)).toBe("112 dmg");
    expect(formatMountMeta(snap.mounts[1]!)).toBe("Laser · Defense");
    expect(formatMountStateLabel(snap.mounts[1]!)).toBe("Defense Screen");
    expect(formatMountMeta(snap.mounts[2]!)).toBe("Tubes · 3/4");
    expect(formatMountMeta(snap.mounts[3]!)).toBe("Spinal · 15/16");
    expect(formatMountStateLabel(snap.mounts[3]!)).toBe("Too Hot");
    expect(formatMountAtLockEffect(snap.mounts[3]!)).toBe("1.8s flight");
    expect(formatMountMeta(snap.mounts[4]!)).toBe("Point Defense · 4000 left");
    expect(formatHeldLockSummary(snap)).toBe(
      "Lock: Red Corsair (corsair.2) · 18.5 km · Strike Band: Rear",
    );
    expect(snap.mining).toEqual({
      matchedRock: "B",
      approachingRock: null,
      activeDrones: 2,
      laserRock: "B",
    });
  });

  it("sends canonical {op:'pack', id} requests on tacsim.content without storming on retries or found:false (C-18)", () => {
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
          panels: [PORT_TRADE_FEED, TACSIM_CONTENT_FEED],
        },
      } as OutputEvent,
      0,
    );

    app.tacsim.ensurePacksSynced();

    // Server sends initial manifest listing two categorized packs (matching client_sync.rs:1102)
    const manifestEvt = {
      t: "panel",
      d: {
        feed: TACSIM_CONTENT_FEED,
        kind: "keyframe",
        data: {
          op: "manifest",
          packs: [
            { id: "neighborhood.ceres_freeport", category: "neighborhood", sha256: "d1" },
            { id: "missing.unit", category: "unit", sha256: "d2" },
          ],
        },
      },
    } as OutputEvent;
    socket.deliver(manifestEvt, 1);

    const parseContentParams = (): Array<Record<string, unknown>> =>
      socket.sent
        .map((raw) => JSON.parse(raw) as Record<string, unknown>)
        .map((msg) => {
          if (msg["t"] === "req" && msg["d"] && typeof msg["d"] === "object") {
            const q = (msg["d"] as Record<string, unknown>)["q"];
            if (q && typeof q === "object") return q as Record<string, unknown>;
          }
          return msg;
        })
        .filter((msg) => msg["t"] === "panel_subscribe")
        .map((msg) => msg["d"] as Record<string, unknown>)
        .filter((d) => d["feed"] === TACSIM_CONTENT_FEED && d["params"] && typeof d["params"] === "object")
        .map((d) => d["params"] as Record<string, unknown>);

    const firstParams = parseContentParams();
    expect(firstParams.some((p) => p["op"] === "manifest")).toBe(true);
    const packReqs = firstParams.filter((p) => p["op"] === "pack");
    expect(packReqs).toEqual([
      { op: "pack", id: "neighborhood.ceres_freeport" },
      { op: "pack", id: "missing.unit" },
    ]);
    // Must NEVER emit the broken legacy `{ pack: id }` shape!
    expect(firstParams.some((p) => "pack" in p)).toBe(false);

    // Repeating the same manifest while packs are in-flight must NOT re-request them
    const sentBeforeDup = socket.sent.length;
    socket.deliver(manifestEvt, 2);
    expect(socket.sent.length).toBe(sentBeforeDup);

    // Server returns `found: false` for `missing.unit` — must NOT retry on subsequent manifests
    socket.deliver(
      {
        t: "panel",
        d: {
          feed: TACSIM_CONTENT_FEED,
          kind: "keyframe",
          data: { op: "pack", id: "missing.unit", found: false },
        },
      } as OutputEvent,
      3,
    );
    const sentAfterNotFound = socket.sent.length;
    socket.deliver(manifestEvt, 4);
    expect(socket.sent.length).toBe(sentAfterNotFound);
  });

  it("clears spaceDockingData when server sends closed or refused docking panel and derives dynamic dock target", () => {
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
        t: "panel",
        d: {
          feed: "space.cockpit",
          kind: "keyframe",
          data: { active: true, site_id: "aitken_highport" },
        },
      } as OutputEvent,
      1,
    );
    expect(app.tacsim.dockTarget).toBe("luna");

    socket.deliver(
      {
        t: "panel",
        d: {
          feed: "space.docking",
          kind: "keyframe",
          data: { active: true, site_id: "aitken_highport", range_m: 120 },
        },
      } as OutputEvent,
      2,
    );
    expect(app.picture.spaceDockingData).not.toBeNull();

    socket.deliver(
      {
        t: "panel",
        d: {
          feed: "space.docking",
          kind: "keyframe",
          data: { closed: true, refused: true },
        },
      } as OutputEvent,
      3,
    );
    expect(app.picture.spaceDockingData).toBeNull();
  });

  it("removes despawned actors cleanly, avoids rubber-band snap on chat/typing, disposes GPU resources, and enforces ship/Berth/UI fixes (W3D-C1/H1/H2/M5/M7/L3)", () => {
    const ceres = parseNeighborhoodPack(
      readRepoContent("atlas/neighborhood.ceres_freeport.json"),
      "neighborhood.ceres_freeport",
    );
    const drayP = parseShipUnitJson(readRepoContent("units/dray_p.json"), "tacsim.unit.dray_p");
    const canvas = document.createElement("canvas");
    let uiBlocked = false;
    const triggered: string[] = [];

    const mockRenderer = () => ({
      setPixelRatio: () => {},
      setSize: () => {},
      render: () => {},
      dispose: () => {},
      toneMapping: 0 as unknown as import("three").ToneMapping,
      toneMappingExposure: 1,
    });

    const stage = new TacsimStage3D(canvas, {
      onSendCommand: () => {},
      onPhysicalLocationChanged: () => {},
      onFocusedPromptChanged: () => {},
      onInteractableTriggered: (wi) => triggered.push(wi.id),
      isUiBlockingInput: () => uiBlocked,
      createRenderer: mockRenderer,
    });

    const intern = stage as unknown as {
      actorsRoot: { children: unknown[] };
      interactables: Array<{ id: string }>;
      shipInteractables: Array<{ id: string }>;
      shipWallObstacles: Array<{ min: { x: number; y: number; z: number }; max: { x: number; y: number; z: number } }>;
      cradleHullGroups: Array<{ visible: boolean }>;
      cargoPalletsGroup: { children: unknown[] } | null;
      holotankContactsGroup: { children: unknown[] } | null;
      pendingMoveTarget: string | null;
      pendingMoveSentAtMs: number;
      camYawRad: number;
      camDist: number;
      contextLost: boolean;
      playerGroup: { position: { x: number; y: number; z: number; set: (x: number, y: number, z: number) => void } };
    };

    const baseAmbient = decodeTacsimRoomAmbient(null);

    // 1. W3D-C1 & W3D-L3: Spawn two actors in Berth Gallery with 0 docked hulls
    stage.syncWorldState({
      stationPack: ceres,
      shipTemplate: drayP,
      isAboardShip: false,
      serverStationRoomEntry: "room.c01_berth_gallery",
      serverShipCompartment: null,
      roomRevision: 1,
      ambient: baseAmbient,
      berthDockedHulls: [],
      berthOwnHullName: null,
      berthOwnHullDockedHere: false,
      visibleActors: [
        { id: "#41", name: "Dock Clerk", isNpc: true, isMale: false, shortDesc: "" },
        { id: "#77", name: "Other Pilot", isNpc: false, isMale: false, shortDesc: "" },
      ],
      actorSpeech: {},
      actorComposing: {},
      stowageCargoKg: 0,
      holotankData: null,
    });

    // 2 actors -> 2 suit roots + 2 billboard sprites = 4 children in actorsRoot
    expect(intern.actorsRoot.children.length).toBe(4);
    expect(intern.interactables.filter((i) => i.id.startsWith("actor:"))).toHaveLength(2);
    // W3D-L3: No ghost hull in Cradle 01 when 0 ships are docked
    expect(intern.cradleHullGroups.length).toBe(4);
    expect(intern.cradleHullGroups.every((g) => g.visible === false)).toBe(true);

    // W3D-H4: Populate cradles live from berthDockedHulls
    stage.syncWorldState({
      stationPack: ceres,
      shipTemplate: drayP,
      isAboardShip: false,
      serverStationRoomEntry: "room.c01_berth_gallery",
      serverShipCompartment: null,
      roomRevision: 1,
      ambient: baseAmbient,
      berthDockedHulls: [
        { id: "#99", name: "Salt Moth", mark: "CF-DR-01", unit: "dray", captain: "", yours: true },
        { id: "#104", name: "Copper Lantern", mark: "CF-DR-02", unit: "dray", captain: "", yours: false },
      ],
      berthOwnHullName: "Salt Moth",
      berthOwnHullDockedHere: true,
      visibleActors: [{ id: "#41", name: "Dock Clerk", isNpc: true, isMale: false, shortDesc: "" }],
      actorSpeech: {},
      actorComposing: {},
      stowageCargoKg: 0,
      holotankData: null,
    });

    // Despawned actor #77 must be fully removed (suit + billboard + interactable)
    expect(intern.actorsRoot.children.length).toBe(2);
    expect(intern.interactables.filter((i) => i.id.startsWith("actor:"))).toHaveLength(1);
    expect(intern.cradleHullGroups[0]?.visible).toBe(true);
    expect(intern.cradleHullGroups[1]?.visible).toBe(true);
    expect(intern.cradleHullGroups[2]?.visible).toBe(false);

    // 2. W3D-H1: Non-movement reactive update (chat/typing) with unchanged roomRevision
    //    must NOT wipe pendingMoveTarget or teleport player / snap camera yaw!
    intern.pendingMoveTarget = "room.c02_concourse";
    intern.pendingMoveSentAtMs = performance.now();
    intern.camYawRad = 1.23;
    intern.playerGroup.position.set(12.5, 0, -4.5);

    stage.syncWorldState({
      stationPack: ceres,
      shipTemplate: drayP,
      isAboardShip: false,
      serverStationRoomEntry: "room.c01_berth_gallery",
      serverShipCompartment: null,
      roomRevision: 1,
      ambient: baseAmbient,
      berthDockedHulls: [
        { id: "#99", name: "Salt Moth", mark: "CF-DR-01", unit: "dray", captain: "", yours: true },
      ],
      visibleActors: [{ id: "#41", name: "Dock Clerk", isNpc: true, isMale: false, shortDesc: "" }],
      actorSpeech: { "#41": { text: "Berth clamps locked.", expiresAt: Date.now() + 8000 } },
      actorComposing: { "#41": true },
      stowageCargoKg: 0,
      holotankData: null,
    });

    expect(intern.pendingMoveTarget).toBe("room.c02_concourse");
    expect(intern.camYawRad).toBeCloseTo(1.23, 5);
    expect(intern.playerGroup.position.x).toBeCloseTo(12.5, 5);

    // Moving to a new room clears all previous actors with zero leaks (W3D-C1)
    stage.syncWorldState({
      stationPack: ceres,
      shipTemplate: drayP,
      isAboardShip: false,
      serverStationRoomEntry: "room.c02_concourse",
      serverShipCompartment: null,
      roomRevision: 2,
      ambient: baseAmbient,
      visibleActors: [],
      actorSpeech: {},
      actorComposing: {},
      stowageCargoKg: 0,
      holotankData: null,
    });
    expect(intern.actorsRoot.children.length).toBe(0);
    expect(intern.interactables.filter((i) => i.id.startsWith("actor:"))).toHaveLength(0);

    // 3. W3D-M5 & W3D-H2: Board Dray-P ship interior, verify sealed engineering/bay & bridge doorway
    const holoFeed = {
      range_km: 1000,
      contacts: [
        { x_km: 250, y_km: 100, z_km: 0, kind: "ship", relation: "hostile" },
        { x_km: -300, y_km: -200, z_km: 50, kind: "asteroid", relation: "neutral" },
      ],
    };
    stage.syncWorldState({
      stationPack: ceres,
      shipTemplate: drayP,
      isAboardShip: true,
      serverStationRoomEntry: null,
      serverShipCompartment: "bridge",
      roomRevision: 3,
      ambient: baseAmbient,
      visibleActors: [],
      actorSpeech: {},
      actorComposing: {},
      stowageCargoKg: 45_000,
      holotankData: holoFeed,
    });

    // Bridge aft wall (local z = 4.5, world y = 160 + 1.8..5.0) must leave BRIDGE_STAIRWELL_XZ
    // (x in -2.65..-0.55) unblocked while sealing both solid side panels.
    const bridgeAftObstacles = intern.shipWallObstacles.filter(
      (b) =>
        b.max.z - b.min.z < 0.5 &&
        b.min.z <= 4.5 &&
        b.max.z >= 4.5 &&
        b.min.y >= 161.5 &&
        b.max.y <= 165.5,
    );
    expect(bridgeAftObstacles).toHaveLength(2);
    for (const box of bridgeAftObstacles) {
      // No solid obstacle may overlap the center of the bridge stairwell doorway (x = -1.6)
      expect(box.min.x > -1.5 || box.max.x < -1.7).toBe(true);
    }

    // Engineering aft bulkhead (local z = -14.0, world y = 160 - 5.3..-1.9) must be sealed solid!
    const engAftWall = intern.shipWallObstacles.find(
      (b) =>
        b.max.z - b.min.z < 0.5 &&
        b.min.z <= -14.0 &&
        b.max.z >= -14.0 &&
        b.min.y <= 155.5 &&
        b.max.y >= 157.5,
    );
    expect(engAftWall).toBeDefined();

    // W3D-H2: Repeated identical cargo/holotank sync keeps pooled meshes without recreating
    const firstCargoMesh = intern.cargoPalletsGroup?.children[0];
    const firstHoloMesh = intern.holotankContactsGroup?.children[0];
    expect(firstCargoMesh).toBeDefined();
    expect(firstHoloMesh).toBeDefined();
    expect(intern.holotankContactsGroup?.children).toHaveLength(3); // self + 2 contacts
    stage.syncWorldState({
      stationPack: ceres,
      shipTemplate: drayP,
      isAboardShip: true,
      serverStationRoomEntry: null,
      serverShipCompartment: "bridge",
      roomRevision: 3,
      ambient: baseAmbient,
      visibleActors: [],
      actorSpeech: {},
      actorComposing: {},
      stowageCargoKg: 45_000,
      holotankData: holoFeed,
    });
    expect(intern.cargoPalletsGroup?.children[0]).toBe(firstCargoMesh);
    expect(intern.holotankContactsGroup?.children[0]).toBe(firstHoloMesh);

    // W3D-M7: UI blocking suppresses wheel zoom and pointer interaction
    uiBlocked = true;
    const distBeforeWheel = intern.camDist;
    canvas.dispatchEvent(new WheelEvent("wheel", { deltaY: 200, cancelable: true }));
    expect(intern.camDist).toBe(distBeforeWheel);
    uiBlocked = false;
    canvas.dispatchEvent(new WheelEvent("wheel", { deltaY: 200, cancelable: true }));
    expect(intern.camDist).toBeGreaterThan(distBeforeWheel);

    // W3D-H2: webglcontextlost prevents default & pauses; webglcontextrestored resumes
    const lostEv = new Event("webglcontextlost", { cancelable: true });
    const preventSpy = vi.spyOn(lostEv, "preventDefault");
    canvas.dispatchEvent(lostEv);
    expect(preventSpy).toHaveBeenCalledTimes(1);
    expect(intern.contextLost).toBe(true);
    canvas.dispatchEvent(new Event("webglcontextrestored"));
    expect(intern.contextLost).toBe(false);

    // Dispose cleans everything cleanly without throwing
    expect(() => stage.dispose()).not.toThrow();
    expect(triggered).toHaveLength(0);
  });
});

describe("PR #693 Web review regressions (items 1-7, 9-11)", () => {
  const makeMockRenderer = () => ({
    setPixelRatio: () => {},
    setSize: () => {},
    render: () => {},
    dispose: () => {},
    toneMapping: 0 as unknown as import("three").ToneMapping,
    toneMappingExposure: 1,
  });

  it("Item 1 [P1]: rejects pre-existing or larger quotes and auto-accepts only a fresh exact quote within authorized max total", () => {
    const sentCommands: string[] = [];
    const store = new TacsimPortStore({
      hasPanel: () => true,
      subscribePanel: () => true,
      sendCommand: (line) => sentCommands.push(line),
      selfCharacterId: () => "#10",
    });

    // Existing open quote #2 for 100 units of listing #41 at 100.00 Cr/t (10,000 Cr total)
    store.observePortTrade({
      venue: "ceres",
      balance_minor: 5_000_000,
      listings: [
        {
          id: 41,
          side: "ask",
          good: "volatiles",
          good_label: "water ice",
          vendor_name: "Occator Ice",
          unit_price_minor: 10_000,
          available_qty: 200,
        },
      ],
      open_quotes: [
        {
          quote_id: 2,
          listing_id: 41,
          quantity: 100,
          unit_price_minor: 10_000,
          total_minor: 1_000_000,
        },
      ],
    });
    const ask = store.trade.asks[0]!;
    expect(store.trade.quotes).toHaveLength(1);

    // User clicks Buy 1 unit on listing #41
    store.buyListing(ask, 1);
    expect(sentCommands).toEqual(["+market/quote 41=1"]);

    // Feed arrives still carrying only old 100-unit quote #2 -> MUST NOT accept #2!
    store.observePortTrade({
      open_quotes: [
        {
          quote_id: 2,
          listing_id: 41,
          quantity: 100,
          unit_price_minor: 10_000,
          total_minor: 1_000_000,
        },
      ],
    });
    expect(sentCommands).toEqual(["+market/quote 41=1"]);

    // Even if a pre-existing quote has the exact same listingId & quantity, it must NOT be accepted
    const preSameQtyLatch = beginOneClickBuy(ask, 1, 1_000, [{ quoteId: 2 }])!.latch;
    const staleSameQtySnap = decodePortTradeFeed(
      {
        open_quotes: [
          { quote_id: 2, listing_id: 41, quantity: 1, unit_price_minor: 10_000, total_minor: 10_000 },
        ],
      },
      store.goodsById,
      store.trade,
    );
    expect(resolvePendingQuoteFromFeed(staleSameQtySnap, preSameQtyLatch, 1_200)).toEqual({
      acceptCommand: null,
      priceWarning: null,
      clearLatch: false,
    });

    // Fresh quote #3 with inflated total_minor (> authorizedMaxTotal) must warn and NOT accept
    const inflatedSnap = decodePortTradeFeed(
      {
        open_quotes: [
          { quote_id: 2, listing_id: 41, quantity: 100, unit_price_minor: 10_000, total_minor: 1_000_000 },
          { quote_id: 3, listing_id: 41, quantity: 1, unit_price_minor: 10_000, total_minor: 15_000 },
        ],
      },
      store.goodsById,
      store.trade,
    );
    const inflatedRes = resolvePendingQuoteFromFeed(inflatedSnap, preSameQtyLatch, 1_200);
    expect(inflatedRes.acceptCommand).toBeNull();
    expect(inflatedRes.priceWarning).toContain("Quote #3 moved");
    expect(inflatedRes.clearLatch).toBe(true);

    // Fresh quote #4 for 1 unit at 10,000 minor total arrives alongside old quote #2 -> accepts ONLY #4!
    store.observePortTrade({
      open_quotes: [
        { quote_id: 2, listing_id: 41, quantity: 100, unit_price_minor: 10_000, total_minor: 1_000_000 },
        { quote_id: 4, listing_id: 41, quantity: 1, unit_price_minor: 10_000, total_minor: 10_000 },
      ],
    });
    expect(sentCommands).toEqual(["+market/quote 41=1", "+market/accept 4"]);
  });

  it("Item 2 [P2]: cold web port immediately swaps Ceres fallback to Stickney Yards when Stickney pack arrives", () => {
    const store = new TacsimPortStore({
      hasPanel: () => true,
      subscribePanel: () => true,
      sendCommand: () => {},
      selfCharacterId: () => "#10",
    });

    // Cold room arrival in Stickney Berth Gallery before Stickney pack has streamed in
    store.observeRoom({
      id: "#201",
      name: "Visiting Berth Gallery",
      desc: [{ text: "Stickney basalt gallery." }],
      contents: [],
      exits: [],
      ambient: {
        atlas: { pack: "neighborhood.stickney_yards", entry: "room.p01_berth_gallery", surface: "station" },
        tacsim: { mode: "berth_gallery", port_entry: "stickney_port" },
      },
    });
    expect(store.serverStationRoomEntry).toBe("room.p01_berth_gallery");
    expect(store.stationPack.packId).toBe("neighborhood.ceres_freeport"); // fallback before pack arrives

    const revBefore = store.packsRevision;
    const stickneyJson = readRepoContent("atlas/neighborhood.stickney_yards.json");
    store.observeContentFeed({
      op: "pack",
      found: true,
      id: "neighborhood.stickney_yards",
      sha256: "sha-stickney-1",
      json: stickneyJson,
    });

    expect(store.stationPack.packId).toBe("neighborhood.stickney_yards");
    expect(store.stationPack.rooms.has("room.p01_berth_gallery")).toBe(true);
    expect(store.stationPack.rooms.has("room.p05_harbourmaster")).toBe(true);
    expect(store.serverStationRoomEntry).toBe("room.p01_berth_gallery");
    expect(store.packsRevision).toBeGreaterThan(revBefore);
  });

  it("Item 3 [P2]: text-mode quick actions follow authoritative server location on station and ship walks", () => {
    const store = new TacsimPortStore({
      hasPanel: () => true,
      subscribePanel: () => true,
      sendCommand: () => {},
      selfCharacterId: () => "#10",
    });
    store.toggle3dView(false);

    // Start in Freeport Concourse
    store.observeRoom({
      id: "#104",
      name: "Freeport Concourse",
      desc: [],
      contents: [],
      exits: [],
      ambient: {
        atlas: { pack: "neighborhood.ceres_freeport", entry: "room.c04_concourse" },
      },
    });
    expect(store.physicalLocationKey).toBe("room.c04_concourse");
    expect(store.quickActions.some((a) => a.label === "Enroll Charter Hull")).toBe(false);

    // Walk within 2 hops to Registry & Co-op Desk in text mode (3D view closed)
    store.observeRoom({
      id: "#105",
      name: "Registry & Co-op Desk",
      desc: [],
      contents: [],
      exits: [],
      ambient: {
        atlas: { pack: "neighborhood.ceres_freeport", entry: "room.c05_registry" },
      },
    });
    expect(store.serverStationRoomEntry).toBe("room.c05_registry");
    expect(store.physicalLocationKey).toBe("room.c05_registry");
    expect(store.quickActions.some((a) => a.label === "Enroll Charter Hull")).toBe(true);

    // Even if physicalLocationKey was left ahead from an earlier 3D walk, closing 3D view uses server room
    store.toggle3dView(true);
    store.physicalLocationKey = "room.c04_concourse";
    expect(store.quickActions.some((a) => a.label === "Enroll Charter Hull")).toBe(false);
    store.toggle3dView(false);
    expect(store.physicalLocationKey).toBe("room.c05_registry");
    expect(store.quickActions.some((a) => a.label === "Enroll Charter Hull")).toBe(true);

    // Same invariant aboard ship: text-mode walk from Airlock -> Bridge switches toolbar to Take Bridge Helm
    store.observeRoom({
      id: "#801",
      name: "Salt Moth - Airlock",
      desc: [],
      contents: [],
      exits: [],
      ambient: { tacsim: { mode: "interior", unit: "dray", room: "airlock", docked: true } },
    });
    expect(store.quickActions.some((a) => a.label === "Disembark Ashore")).toBe(true);
    store.observeRoom({
      id: "#802",
      name: "Salt Moth - Bridge",
      desc: [],
      contents: [],
      exits: [],
      ambient: { tacsim: { mode: "interior", unit: "dray", room: "bridge", docked: true } },
    });
    expect(store.physicalLocationKey).toBe("bridge");
    expect(store.quickActions.some((a) => a.label === "Take Bridge Helm")).toBe(true);
  });

  it("Item 4 [P2]: failed WebGL initialization does not capture WASD, Shift, or Arrow keys", () => {
    const canvas = document.createElement("canvas");
    const failedStage = new TacsimStage3D(canvas, {
      onSendCommand: () => {},
      onPhysicalLocationChanged: () => {},
      onFocusedPromptChanged: () => {},
      onInteractableTriggered: () => {},
      isUiBlockingInput: () => false,
      createRenderer: () => {
        throw new Error("WebGL context creation failed");
      },
    });
    expect(failedStage.supported).toBe(false);

    const bubbleReceived: string[] = [];
    const onBubble = (ev: KeyboardEvent): void => {
      bubbleReceived.push(ev.code);
    };
    window.addEventListener("keydown", onBubble);
    try {
      for (const code of ["ArrowUp", "ArrowDown", "KeyW", "KeyA", "ShiftLeft"]) {
        const ev = new KeyboardEvent("keydown", { code, cancelable: true, bubbles: true });
        window.dispatchEvent(ev);
        expect(ev.defaultPrevented).toBe(false);
      }
      expect(bubbleReceived).toEqual(["ArrowUp", "ArrowDown", "KeyW", "KeyA", "ShiftLeft"]);
    } finally {
      window.removeEventListener("keydown", onBubble);
      failedStage.dispose();
    }

    // Contrast: a supported stage DOES capture movement keys while mounted and releases on dispose
    const liveStage = new TacsimStage3D(canvas, {
      onSendCommand: () => {},
      onPhysicalLocationChanged: () => {},
      onFocusedPromptChanged: () => {},
      onInteractableTriggered: () => {},
      isUiBlockingInput: () => false,
      createRenderer: makeMockRenderer,
    });
    expect(liveStage.supported).toBe(true);
    const activeEv = new KeyboardEvent("keydown", { code: "ArrowUp", cancelable: true, bubbles: true });
    window.dispatchEvent(activeEv);
    expect(activeEv.defaultPrevented).toBe(true);
    liveStage.dispose();
    const afterDisposeEv = new KeyboardEvent("keydown", { code: "ArrowUp", cancelable: true, bubbles: true });
    window.dispatchEvent(afterDisposeEv);
    expect(afterDisposeEv.defaultPrevented).toBe(false);
  });

  it("Item 5 [P2]: resizing TacsimSceneDeck viewport resizes the live Stage3D without disposing or recreating it", async () => {
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
        d: { proto: 1, server: "test", features: [], panels: [PORT_TRADE_FEED, TACSIM_CONTENT_FEED] },
      } as OutputEvent,
      0,
    );
    app.tacsim.toggle3dView(true);

    const resizeSpy = vi.spyOn(TacsimStage3D.prototype, "resize");
    const disposeSpy = vi.spyOn(TacsimStage3D.prototype, "dispose");

    const { container, unmount } = render(TacsimSceneDeck, {
      props: { app, onCmd: () => {} },
    });
    await tick();

    expect(disposeSpy).toHaveBeenCalledTimes(0);
    const resizeCallsAfterMount = resizeSpy.mock.calls.length;
    expect(resizeCallsAfterMount).toBeGreaterThanOrEqual(1);

    // Drag the resize separator across multiple heights
    const grip = container.querySelector(".resize-grip") as HTMLElement;
    expect(grip).not.toBeNull();
    await fireEvent.pointerDown(grip, { clientY: 400, pointerId: 1 });
    await fireEvent.pointerMove(grip, { clientY: 460, pointerId: 1 });
    await tick();
    await fireEvent.pointerMove(grip, { clientY: 520, pointerId: 1 });
    await tick();
    await fireEvent.pointerUp(grip, { clientY: 520, pointerId: 1 });
    await tick();

    // Stage must NEVER have been disposed/recreated during resize!
    expect(disposeSpy).toHaveBeenCalledTimes(0);
    expect(app.tacsim.viewportHeightPx).toBe(500);
    expect(resizeSpy.mock.calls.length).toBeGreaterThan(resizeCallsAfterMount);
    expect(resizeSpy.mock.calls.some((call) => call[1] === 500)).toBe(true);

    unmount();
    expect(disposeSpy).toHaveBeenCalledTimes(1);
    resizeSpy.mockRestore();
    disposeSpy.mockRestore();
  });

  it("Items 6 & 7 [P2]: holotank plots contacts in observer-relative frame and renders no fabricated contacts when empty or unavailable", () => {
    const ceres = parseNeighborhoodPack(
      readRepoContent("atlas/neighborhood.ceres_freeport.json"),
      "neighborhood.ceres_freeport",
    );
    const dray = parseShipUnitJson(readRepoContent("units/dray.json"), "tacsim.unit.dray");
    const canvas = document.createElement("canvas");
    const stage = new TacsimStage3D(canvas, {
      onSendCommand: () => {},
      onPhysicalLocationChanged: () => {},
      onFocusedPromptChanged: () => {},
      onInteractableTriggered: () => {},
      isUiBlockingInput: () => false,
    });

    const intern = stage as unknown as {
      holotankContactsGroup: {
        children: Array<{
          position: { x: number; y: number; z: number };
          userData: Record<string, unknown>;
        }>;
      } | null;
      holotankStatusBanner: { lastText: string } | null;
    };
    const baseAmbient = decodeTacsimRoomAmbient(null);
    const syncBridge = (holotankData: unknown): void => {
      stage.syncWorldState({
        stationPack: ceres,
        shipTemplate: dray,
        isAboardShip: true,
        serverStationRoomEntry: null,
        serverShipCompartment: "bridge",
        roomRevision: 1,
        ambient: baseAmbient,
        visibleActors: [],
        actorSpeech: {},
        actorComposing: {},
        stowageCargoKg: 0,
        holotankData,
      });
    };

    // Item 7: Missing / null / refused feed produces ZERO fabricated blips
    syncBridge(null);
    expect(intern.holotankContactsGroup?.children).toHaveLength(0);
    expect(intern.holotankStatusBanner?.lastText).toContain("NO TELEMETRY");

    syncBridge({ refused: true });
    expect(intern.holotankContactsGroup?.children).toHaveLength(0);

    // Item 7: Empty scan (0 other contacts) renders ONLY own-ship center blip, NO dummy ship/asteroid
    syncBridge({
      range_km: 250,
      contacts: [{ id: "self", you: true, x_km: 500, y_km: -300, z_km: 50 }],
    });
    expect(intern.holotankContactsGroup?.children).toHaveLength(1);
    expect(intern.holotankContactsGroup?.children[0]?.userData["holoKind"]).toBe("self");
    expect(intern.holotankStatusBanner?.lastText).toContain("0 CONTACTS");

    // Item 6: Nearby contact in bubble-absolute coordinates is plotted relative to observer (`you: true`)
    syncBridge({
      range_km: 250,
      contacts: [
        { id: "self", you: true, x_km: 500, y_km: -300, z_km: 50, kind: "ship" },
        { id: "raider", you: false, x_km: 525, y_km: -250, z_km: 75, kind: "ship", relation: "hostile" },
      ],
    });
    expect(intern.holotankContactsGroup?.children).toHaveLength(2);
    const selfBlip = intern.holotankContactsGroup!.children[0]!;
    const raiderBlip = intern.holotankContactsGroup!.children[1]!;
    expect(selfBlip.position).toMatchObject({ x: 0, y: 0, z: 0 });
    // Relative delta = (+25, +50, +25) km / 250 km => x=0.065, z=0.13, y=0.045
    expect(raiderBlip.position.x).toBeCloseTo((25 / 250) * 0.65, 5);
    expect(raiderBlip.position.z).toBeCloseTo((50 / 250) * 0.65, 5);
    expect(raiderBlip.position.y).toBeCloseTo((25 / 250) * 0.45, 5);

    stage.dispose();
  });

  it("Item 9 [P2]: failed sends are not stuck in flight and timed-out/throttled pack requests retry with bounded backoff", () => {
    vi.useFakeTimers();
    try {
      let allowSend = false;
      const requestedPacks: string[] = [];
      const store = new TacsimPortStore({
        hasPanel: () => true,
        subscribePanel: (_feed, params) => {
          if (params?.["op"] === "pack" && typeof params["id"] === "string") {
            requestedPacks.push(params["id"]);
          }
          return allowSend;
        },
        sendCommand: () => {},
        selfCharacterId: () => "#10",
      });

      // 1. Failed send (`subscribePanel` returns false) must NOT lock ID in flight forever
      store.observeContentFeed({
        op: "manifest",
        packs: [{ id: "goods", category: "market", sha256: "g-1" }],
      });
      expect(requestedPacks).toEqual(["goods"]);

      // Transport recovers before retry timer fires -> retry sends successfully
      allowSend = true;
      vi.advanceTimersByTime(PACK_RETRY_BASE_DELAY_MS);
      expect(requestedPacks).toEqual(["goods", "goods"]);

      // 2. Sent successfully (`true`), but server throttles/drops the reply:
      //    duplicate manifest within timeout does NOT spam
      store.observeContentFeed({
        op: "manifest",
        packs: [{ id: "goods", category: "market", sha256: "g-1" }],
      });
      expect(requestedPacks).toHaveLength(2);

      //    Once in-flight timeout elapses + backoff fires, store retries automatically and bounds retries
      vi.advanceTimersByTime(60_000);
      // 1 failed attempt (retry #1) + 1 sent attempt + remaining retries up to PACK_MAX_RETRIES
      expect(requestedPacks.length).toBe(PACK_MAX_RETRIES + 1);

      // No further sends after bounded retry budget is exhausted for digest g-1
      const countAtCap = requestedPacks.length;
      vi.advanceTimersByTime(120_000);
      expect(requestedPacks.length).toBe(countAtCap);

      // A later manifest with a new digest (g-2) resets retry budget and refetches immediately
      store.observeContentFeed({
        op: "manifest",
        packs: [{ id: "goods", category: "market", sha256: "g-2" }],
      });
      expect(requestedPacks.length).toBe(countAtCap + 1);
      expect(requestedPacks[requestedPacks.length - 1]).toBe("goods");
      store.observeContentFeed({
        op: "pack",
        found: true,
        id: "goods",
        sha256: "g-2",
        json: readRepoContent("market/goods.json"),
      });

      // Separate pack: verify timeout expiry -> retry -> successful delivery stops retries
      requestedPacks.length = 0;
      store.observeContentFeed({
        op: "manifest",
        packs: [{ id: "dray", category: "unit", sha256: "d-1" }],
      });
      expect(requestedPacks).toEqual(["dray"]);
      vi.advanceTimersByTime(PACK_IN_FLIGHT_TIMEOUT_MS + PACK_RETRY_BASE_DELAY_MS);
      expect(requestedPacks).toEqual(["dray", "dray"]);

      // Deliver reply -> stops all retries
      store.observeContentFeed({
        op: "pack",
        found: true,
        id: "dray",
        sha256: "d-1",
        json: readRepoContent("units/dray.json"),
      });
      vi.advanceTimersByTime(60_000);
      expect(requestedPacks).toEqual(["dray", "dray"]);

      store.clear();
    } finally {
      vi.useRealTimers();
    }
  });

  it("Item 10 [P2]: compares manifest digests, invalidates stale packs, and resets raw + derived caches across sessions/characters", () => {
    let charId = "#10";
    const requestedPacks: string[] = [];
    const store = new TacsimPortStore({
      hasPanel: () => true,
      subscribePanel: (_feed, params) => {
        if (params?.["op"] === "pack" && typeof params["id"] === "string") {
          requestedPacks.push(params["id"]);
        }
        return true;
      },
      sendCommand: () => {},
      selfCharacterId: () => charId,
    });

    const goodsV1 = JSON.stringify({
      goods: [
        {
          id: "metals",
          label: "ore v1",
          class: "trade",
          ledger_unit: "t",
          measures: { "si.mass_kg": 1000, "si.volume_l": 400 },
          stowage: "stow.bulk",
          reference_major: 200,
        },
      ],
    });
    const goodsV2 = JSON.stringify({
      goods: [
        {
          id: "metals",
          label: "ore v2",
          class: "trade",
          ledger_unit: "t",
          measures: { "si.mass_kg": 2500, "si.volume_l": 900 },
          stowage: "stow.bulk",
          reference_major: 350,
        },
      ],
    });

    // 1. Initial manifest with digest A -> fetches & caches goods v1
    store.observeContentFeed({
      op: "manifest",
      packs: [{ id: "goods", category: "market", sha256: "digest-A" }],
    });
    expect(requestedPacks).toEqual(["goods"]);
    store.observeContentFeed({
      op: "pack",
      found: true,
      id: "goods",
      sha256: "digest-A",
      json: goodsV1,
    });
    expect(store.goodsById.get("metals")?.label).toBe("ore v1");

    // 2. Identical digest A -> zero refetches
    store.observeContentFeed({
      op: "manifest",
      packs: [{ id: "goods", category: "market", sha256: "digest-A" }],
    });
    expect(requestedPacks).toEqual(["goods"]);

    // 3. Updated manifest with digest B -> invalidates stale cache and re-fetches!
    store.observeContentFeed({
      op: "manifest",
      packs: [{ id: "goods", category: "market", sha256: "digest-B" }],
    });
    expect(requestedPacks).toEqual(["goods", "goods"]);
    // Out-of-order stale digest-A reply must be rejected
    store.observeContentFeed({
      op: "pack",
      found: true,
      id: "goods",
      sha256: "digest-A",
      json: goodsV1,
    });
    expect(store.goodsById.get("metals")?.label).toBe("ore v1");
    // Fresh digest-B reply updates derived goods table
    store.observeContentFeed({
      op: "pack",
      found: true,
      id: "goods",
      sha256: "digest-B",
      json: goodsV2,
    });
    expect(store.goodsById.get("metals")?.label).toBe("ore v2");
    expect(store.goodsById.get("metals")?.massKgPerUnit).toBe(2500);

    // 4. Clearing session resets BOTH raw and derived caches; receiving digest-B again fetches fresh!
    store.clear();
    expect(store.goodsById.get("metals")?.label).toBe("metal concentrate"); // fallback restored
    requestedPacks.length = 0;
    store.observeContentFeed({
      op: "manifest",
      packs: [{ id: "goods", category: "market", sha256: "digest-B" }],
    });
    expect(requestedPacks).toEqual(["goods"]);
    store.observeContentFeed({
      op: "pack",
      found: true,
      id: "goods",
      sha256: "digest-B",
      json: goodsV2,
    });
    expect(store.goodsById.get("metals")?.label).toBe("ore v2");

    // 5. Switching character automatically partitions/resets raw & derived pack caches
    charId = "#20";
    requestedPacks.length = 0;
    store.observeContentFeed({
      op: "manifest",
      packs: [{ id: "goods", category: "market", sha256: "digest-B" }],
    });
    expect(store.goodsById.get("metals")?.label).toBe("metal concentrate");
    expect(requestedPacks).toEqual(["goods"]);
  });

  it("Item 11 [P2]: arriving ship pack replaces fallback geometry for a cold Tycho Lifter and builds its vehicle bay", () => {
    const store = new TacsimPortStore({
      hasPanel: () => true,
      subscribePanel: () => true,
      sendCommand: () => {},
      selfCharacterId: () => "#10",
    });

    // Cold room arrival aboard Tycho Lifter before `tycho_lifter` unit pack has arrived
    store.observeRoom({
      id: "#901",
      name: "Iron Anchor - Airlock",
      desc: [],
      contents: [],
      exits: [],
      ambient: {
        tacsim: {
          mode: "interior",
          unit: "tacsim.unit.tycho_lifter",
          room: "airlock",
          hull_name: "Iron Anchor",
          docked: true,
        },
      },
    });
    expect(store.shipTemplate.unitId).toBe("tacsim.unit.tycho_lifter");
    expect(store.shipTemplate.isFallback).toBe(true);
    expect(store.shipTemplate.compartments.has("bay")).toBe(false);

    const canvas = document.createElement("canvas");
    const stage = new TacsimStage3D(canvas, {
      onSendCommand: () => {},
      onPhysicalLocationChanged: () => {},
      onFocusedPromptChanged: () => {},
      onInteractableTriggered: () => {},
      isUiBlockingInput: () => false,
    });
    const intern = stage as unknown as {
      shipInteractables: Array<{ id: string }>;
    };

    stage.syncWorldState({
      stationPack: store.stationPack,
      shipTemplate: store.shipTemplate,
      isAboardShip: true,
      serverStationRoomEntry: null,
      serverShipCompartment: "airlock",
      roomRevision: store.roomRevision,
      ambient: store.ambient,
      visibleActors: [],
      actorSpeech: {},
      actorComposing: {},
      stowageCargoKg: 0,
      holotankData: null,
    });
    // Fallback 6-room interior has no vehicle bay rigs
    expect(intern.shipInteractables.filter((i) => i.id.startsWith("ship:rig:"))).toHaveLength(0);

    // Real Tycho Lifter pack arrives over `tacsim.content`
    store.observeContentFeed({
      op: "pack",
      found: true,
      id: "tycho_lifter",
      sha256: "sha-tycho-1",
      json: readRepoContent("units/tycho_lifter.json"),
    });
    expect(store.shipTemplate.unitId).toBe("tacsim.unit.tycho_lifter");
    expect(store.shipTemplate.isFallback).toBe(false);
    expect(store.shipTemplate.compartments.has("bay")).toBe(true);

    // Next syncWorldState MUST rebuild ship interior despite unchanged unitId!
    stage.syncWorldState({
      stationPack: store.stationPack,
      shipTemplate: store.shipTemplate,
      isAboardShip: true,
      serverStationRoomEntry: null,
      serverShipCompartment: "airlock",
      roomRevision: store.roomRevision,
      ambient: store.ambient,
      visibleActors: [],
      actorSpeech: {},
      actorComposing: {},
      stowageCargoKg: 0,
      holotankData: null,
    });
    expect(intern.shipInteractables.filter((i) => i.id.startsWith("ship:rig:"))).toHaveLength(2);
    stage.dispose();
  });

  it("disposes actor meshes/materials on despawn and uses prototype-safe lookups for actorComposing and actorSpeech", () => {
    const ceres = parseNeighborhoodPack(
      readRepoContent("atlas/neighborhood.ceres_freeport.json"),
      "neighborhood.ceres_freeport",
    );
    const drayP = parseShipUnitJson(readRepoContent("units/dray_p.json"), "tacsim.unit.dray_p");
    const baseAmbient = decodeTacsimRoomAmbient(null);
    const canvas = document.createElement("canvas");
    const stage = new TacsimStage3D(canvas, {
      onSendCommand: () => {},
      onPhysicalLocationChanged: () => {},
      onFocusedPromptChanged: () => {},
      onInteractableTriggered: () => {},
      isUiBlockingInput: () => false,
    });
    const intern = stage as unknown as {
      spawnedActors: Map<
        string,
        {
          group: { userData: Record<string, unknown>; traverse: (fn: (o: { userData: Record<string, unknown> }) => void) => void };
          billboardSprite: { userData: Record<string, unknown> };
          lastIsTyping: boolean;
          speechExpiresAt: number;
        }
      >;
      interactables: Array<{ id: string }>;
    };

    stage.syncWorldState({
      stationPack: ceres,
      shipTemplate: drayP,
      isAboardShip: false,
      serverStationRoomEntry: "room.c01_berth_gallery",
      serverShipCompartment: null,
      roomRevision: 1,
      ambient: baseAmbient,
      visibleActors: [
        { id: "#501", name: "Constructor", isNpc: false, isMale: false, shortDesc: "" },
        { id: "#502", name: "toString", isNpc: false, isMale: false, shortDesc: "" },
        { id: "#503", name: "__proto__", isNpc: false, isMale: false, shortDesc: "" },
      ],
      actorSpeech: {},
      actorComposing: {},
      stowageCargoKg: 0,
      holotankData: null,
    });

    // Prototype properties on empty plain records must never evaluate truthy.
    for (const id of ["#501", "#502", "#503"]) {
      const rec = intern.spawnedActors.get(id)!;
      expect(rec.lastIsTyping).toBe(false);
      expect(rec.speechExpiresAt).toBe(0);
    }

    // Track mesh/sprite nodes of #501, then despawn it and verify disposal + interactable cleanup.
    const rec501 = intern.spawnedActors.get("#501")!;
    const visitedNodes: Array<{ userData: Record<string, unknown> }> = [];
    rec501.group.traverse((o) => visitedNodes.push(o));
    expect(visitedNodes.length).toBeGreaterThan(1);

    stage.syncWorldState({
      stationPack: ceres,
      shipTemplate: drayP,
      isAboardShip: false,
      serverStationRoomEntry: "room.c01_berth_gallery",
      serverShipCompartment: null,
      roomRevision: 1,
      ambient: baseAmbient,
      visibleActors: [{ id: "#502", name: "toString", isNpc: false, isMale: false, shortDesc: "" }],
      actorSpeech: {},
      actorComposing: { "#502": true },
      stowageCargoKg: 0,
      holotankData: null,
    });

    expect(visitedNodes.every((n) => n.userData["disposed"] === true)).toBe(true);
    expect(rec501.billboardSprite.userData["disposed"]).toBe(true);
    expect(intern.interactables.some((i) => i.id === "actor:#501")).toBe(false);
    expect(intern.spawnedActors.get("#502")!.lastIsTyping).toBe(true);

    // TacsimPortStore.observeNarrative must not mutate prototype when actor name is "__proto__".
    const store = new TacsimPortStore({
      sendCommand: () => {},
      subscribePanel: () => true,
      hasPanel: () => true,
      selfCharacterId: () => "#1",
    });
    store.observeNarrative({
      kind: "say",
      actor: { id: "#503", name: "__proto__" },
      text: "hello",
    });
    expect(Object.getPrototypeOf(store.actorSpeech)).toBe(Object.prototype);
    expect(Object.hasOwn(store.actorSpeech, "__proto__")).toBe(true);
    stage.dispose();
  });

  it("EXT-P1-6: prefixes all authored exits with goto, rejects blank/control names, and preserves static +tv/bay actions", () => {
    // 1. Direct formatter checks
    expect(formatStationExitCommand("n")).toBe("goto n");
    expect(formatStationExitCommand("@destroy me")).toBe("goto @destroy me");
    expect(formatStationExitCommand("QUIT")).toBe("goto QUIT");
    expect(formatStationExitCommand("+tv/bay in")).toBe("goto +tv/bay in");
    expect(formatStationExitCommand("")).toBeNull();
    expect(formatStationExitCommand("   ")).toBeNull();
    expect(formatStationExitCommand("North\nQUIT")).toBeNull();
    expect(formatStationExitCommand("North\r\n@boot")).toBeNull();
    expect(formatStationExitCommand("North\u009b2J")).toBeNull();

    // Ship static actions are ONLY allowlisted when explicitly flagged isStaticAction=true.
    expect(formatShipTraversalCommand("+tv/bay in", true)).toBe("+tv/bay in");
    expect(formatShipTraversalCommand("+tv/bay out", true)).toBe("+tv/bay out");
    expect(formatShipTraversalCommand("+tv/bay in", false)).toBe("goto +tv/bay in");
    expect(formatShipTraversalCommand("@destroy me", true)).toBeNull();

    // 2. Hostile authored station neighborhood pack
    const hostileStation = parseNeighborhoodPack(
      JSON.stringify({
        manifest: { id: "neighborhood.hostile_test", version: 1, kind: "neighborhood" },
        entries: {
          "room.a": { kind: "room", def: { name: "A", footprint: { rect: [[0, 0], [10, 10]] } } },
          "room.b": { kind: "room", def: { name: "B", footprint: { rect: [[10, 0], [20, 10]] } } },
          "room.c": { kind: "room", def: { name: "C", footprint: { rect: [[20, 0], [30, 10]] } } },
          "room.d": { kind: "room", def: { name: "D", footprint: { rect: [[30, 0], [40, 10]] } } },
          "exit.ab": {
            kind: "exit",
            def: { from: "room.a", to: "room.b", names: ["@destroy me", "QUIT"] },
          },
          "exit.bc_ctrl": {
            kind: "exit",
            def: { from: "room.b", to: "room.c", names: ["East;e\nQUIT", "West;w"] },
          },
          "exit.bd_blank": {
            kind: "exit",
            def: { from: "room.b", to: "room.d", names: ["   ", ""] },
          },
        },
      }),
      "neighborhood.hostile_test",
    );
    expect(findStationExitPath(hostileStation, "room.a", "room.b")).toEqual([
      { command: "goto @destroy me", targetRoom: "room.b" },
    ]);
    expect(findStationExitPath(hostileStation, "room.b", "room.a")).toEqual([
      { command: "goto QUIT", targetRoom: "room.a" },
    ]);
    // Control-char and blank exits are completely untraversable
    expect(findStationExitPath(hostileStation, "room.b", "room.c")).toEqual([]);
    expect(findStationExitPath(hostileStation, "room.b", "room.d")).toEqual([]);

    // 3. Real Ceres Freeport station paths prefix every hop with `goto `
    const ceres = parseNeighborhoodPack(
      readRepoContent("atlas/neighborhood.ceres_freeport.json"),
      "neighborhood.ceres_freeport",
    );
    const ceresPath = findStationExitPath(ceres, "room.c01_berth_gallery", "room.c07_exchange");
    expect(ceresPath.length).toBeGreaterThanOrEqual(2);
    for (const step of ceresPath) {
      expect(step.command.startsWith("goto ")).toBe(true);
    }

    // 4. Dray-P ship interior: authored hops use `goto `, while static hold <-> bay keeps `+tv/bay in|out`,
    //    and a maliciously authored `+tv/bay in` inside `interior.exits` is prefixed with `goto `!
    const drayP = parseShipUnitJson(readRepoContent("units/dray_p.json"), "tacsim.unit.dray_p");
    const toBay = findShipExitPath(drayP, "airlock", "bay");
    expect(toBay.length).toBeGreaterThanOrEqual(3);
    for (let i = 0; i < toBay.length - 1; i++) {
      expect(toBay[i]!.command.startsWith("goto ")).toBe(true);
    }
    expect(toBay[toBay.length - 1]).toEqual({ command: "+tv/bay in", targetCompartment: "bay" });
    expect(findShipExitPath(drayP, "bay", "hold")).toEqual([
      { command: "+tv/bay out", targetCompartment: "hold" },
    ]);

    const hostileShip = parseShipUnitJson(
      JSON.stringify({
        entries: {
          "tacsim.unit.evil": {
            kind: "unit",
            def: {
              label: "Evil Hull",
              interior: {
                rooms: [
                  { id: "airlock", kind: "walkable", name: "Airlock" },
                  { id: "passage", kind: "walkable", name: "Passage" },
                  { id: "bridge", kind: "walkable", name: "Bridge" },
                ],
                exits: [
                  { from: "airlock", to: "passage", names: ["Fake Bay;+tv/bay in", "Back;fore"] },
                  { from: "passage", to: "bridge", names: ["Up;u\n@destroy me", "Down;d"] },
                ],
              },
            },
          },
        },
      }),
      "tacsim.unit.evil",
    );
    expect(findShipExitPath(hostileShip, "airlock", "passage")).toEqual([
      { command: "goto +tv/bay in", targetCompartment: "passage" },
    ]);
    expect(findShipExitPath(hostileShip, "passage", "bridge")).toEqual([]);
  });

  it("EXT-P1-6: RoomBoundNavWalker and Stage3D flushPendingMovesBeforeAction dispatch strictly one room-bound hop at a time", () => {
    // 1. Direct RoomBoundNavWalker state-machine tests
    const walker = new RoomBoundNavWalker();
    const hops = [
      { command: "goto n", targetRoom: "room.b" },
      { command: "goto e", targetRoom: "room.c" },
      { command: "goto n", targetRoom: "room.d" },
    ];
    const h1 = walker.planAndDispatchOne("station", "room.a", hops, 1000);
    expect(h1).toEqual({
      scope: "station",
      fromRoom: "room.a",
      targetRoom: "room.b",
      command: "goto n",
    });
    expect(walker.queuedHops).toEqual([
      { scope: "station", fromRoom: "room.b", targetRoom: "room.c", command: "goto e" },
      { scope: "station", fromRoom: "room.c", targetRoom: "room.d", command: "goto n" },
    ]);
    // Second plan while hop 1 is still in flight MUST NOT dispatch anything
    expect(walker.planAndDispatchOne("station", "room.a", hops, 1200)).toBeNull();

    // Matching authoritative update confirms room.b and advances ONLY hop 2 (bound to fromRoom=room.b)
    const r2 = walker.onAuthoritativeRoomUpdate("station", "room.b", 1300, true);
    expect(r2.acknowledgedTarget).toBe("room.b");
    expect(r2.canceledOnMismatch).toBe(false);
    expect(r2.nextHopToSend).toEqual({
      scope: "station",
      fromRoom: "room.b",
      targetRoom: "room.c",
      command: "goto e",
    });
    expect(walker.queuedCount).toBe(1);

    // Mismatched authoritative update (e.g. exit blocked / redirected to room.trap) cancels remaining queue
    const rBad = walker.onAuthoritativeRoomUpdate("station", "room.trap", 1500, true);
    expect(rBad.acknowledgedTarget).toBeNull();
    expect(rBad.nextHopToSend).toBeNull();
    expect(rBad.canceledOnMismatch).toBe(true);
    expect(walker.inFlight).toBeNull();
    expect(walker.queuedCount).toBe(0);

    // 2. End-to-end Stage3D multi-hop flush: never blasts multiple hops, advances one-by-one on RoomView,
    //    and cancels remaining hops on mismatch or explicit action.
    const ceres = parseNeighborhoodPack(
      readRepoContent("atlas/neighborhood.ceres_freeport.json"),
      "neighborhood.ceres_freeport",
    );
    const drayP = parseShipUnitJson(readRepoContent("units/dray_p.json"), "tacsim.unit.dray_p");
    const baseAmbient = decodeTacsimRoomAmbient(null);
    const sent: string[] = [];
    const canvas = document.createElement("canvas");
    const stage = new TacsimStage3D(canvas, {
      onSendCommand: (cmd) => sent.push(cmd),
      onPhysicalLocationChanged: () => {},
      onFocusedPromptChanged: () => {},
      onInteractableTriggered: () => {},
      isUiBlockingInput: () => false,
    });
    const intern = stage as unknown as {
      originOffsetX: number;
      originOffsetZ: number;
      playerGroup: { position: { set: (x: number, y: number, z: number) => void } };
    };

    const syncStation = (roomEntry: string, rev: number): void => {
      stage.syncWorldState({
        stationPack: ceres,
        shipTemplate: drayP,
        isAboardShip: false,
        serverStationRoomEntry: roomEntry,
        serverShipCompartment: null,
        roomRevision: rev,
        ambient: baseAmbient,
        visibleActors: [],
        actorSpeech: {},
        actorComposing: {},
        stowageCargoKg: 0,
        holotankData: null,
      });
    };

    // Place server in room.c01_berth_gallery, physically move player 2 hops north into room.c03_arrivals
    syncStation("room.c01_berth_gallery", 1);
    const arrivals = ceres.rooms.get("room.c03_arrivals")!;
    const elev = getFloorElevationY(ceres, arrivals.floorIndex);
    const arrivalsWorld = atlasToWorldPos(
      (arrivals.minX + arrivals.maxX) * 0.5,
      elev,
      (arrivals.minY + arrivals.maxY) * 0.5,
      intern.originOffsetX,
      intern.originOffsetZ,
    );
    intern.playerGroup.position.set(arrivalsWorld.x, arrivalsWorld.y, arrivalsWorld.z);

    // Calling flushPendingMovesBeforeAction twice BEFORE confirmation must send ONLY hop 1 (`goto n`)
    stage.flushPendingMovesBeforeAction(10_000);
    stage.flushPendingMovesBeforeAction(10_100);
    expect(sent).toEqual(["goto n"]);

    // Authoritative RoomView confirming room.c02_pressure advances ONLY hop 2 (`goto n`)
    syncStation("room.c02_pressure", 2);
    expect(sent).toEqual(["goto n", "goto n"]);

    // Authoritative RoomView confirming room.c03_arrivals completes with no extra commands
    syncStation("room.c03_arrivals", 3);
    expect(sent).toEqual(["goto n", "goto n"]);

    // Now test mismatch cancellation: walk 2 hops back south to room.c01_berth_gallery, flush hop 1,
    // then server sends authoritative RoomView STILL in room.c03_arrivals (blocked exit) -> hop 2 MUST NOT send!
    sent.length = 0;
    const gallery = ceres.rooms.get("room.c01_berth_gallery")!;
    const galleryWorld = atlasToWorldPos(
      (gallery.minX + gallery.maxX) * 0.5,
      getFloorElevationY(ceres, gallery.floorIndex),
      (gallery.minY + gallery.maxY) * 0.5,
      intern.originOffsetX,
      intern.originOffsetZ,
    );
    intern.playerGroup.position.set(galleryWorld.x, galleryWorld.y, galleryWorld.z);
    stage.flushPendingMovesBeforeAction(20_000);
    expect(sent).toEqual(["goto s"]);
    syncStation("room.c03_arrivals", 4);
    expect(sent).toEqual(["goto s"]);

    // Now test explicit action cancellation: start a 2-hop flush, then notifyExplicitCommandSent() cancels hop 2
    sent.length = 0;
    intern.playerGroup.position.set(galleryWorld.x, galleryWorld.y, galleryWorld.z);
    stage.flushPendingMovesBeforeAction(30_000);
    expect(sent).toEqual(["goto s"]);
    stage.notifyExplicitCommandSent();
    syncStation("room.c02_pressure", 5);
    expect(sent).toEqual(["goto s"]);

    stage.dispose();
  });

  it("renders 3D NPC quest markers, enriches [E] prompts, and hands off available/turn-in NPCs to Quest Dialog", async () => {
    // 1. Pure prompt formatting across all 4 marker states
    expect(
      formatActorInteractPrompt("Maren Halvorsen", true, "Steward", {
        npcId: "#42",
        marker: "available",
        symbol: "!",
        questTitle: "First Papers at the Registry",
      }),
    ).toEqual({
      title: "[!] Maren Halvorsen — Quest Available",
      subtitle: "Offer: First Papers at the Registry",
    });
    expect(
      formatActorInteractPrompt("Ilse Oduya-Brandt", true, "Clerk", {
        npcId: "#55",
        marker: "turn_in",
        symbol: "?",
        questTitle: "First Papers at the Registry",
      }),
    ).toEqual({
      title: "[?] Ilse Oduya-Brandt — Turn In Quest",
      subtitle: "Complete: First Papers at the Registry",
    });
    expect(
      formatActorInteractPrompt("Ilse Oduya-Brandt", true, "Clerk", {
        npcId: "#55",
        marker: "in_progress",
        symbol: "?",
        questTitle: "Stock the Locker and Tanks",
      }),
    ).toEqual({
      title: "[?] Talk to Ilse Oduya-Brandt",
      subtitle: "Active quest: Stock the Locker and Tanks",
    });
    expect(formatActorInteractPrompt("Maren Halvorsen", true, "Steward", null)).toEqual({
      title: "Talk to Maren Halvorsen",
      subtitle: "Steward",
    });

    // 2. Stage3D syncs live roomMarkers onto spawned NPCs and updates [E] prompt live
    const ceres = parseNeighborhoodPack(
      readRepoContent("atlas/neighborhood.ceres_freeport.json"),
      "neighborhood.ceres_freeport",
    );
    const dray = parseShipUnitJson(readRepoContent("units/dray.json"), "tacsim.unit.dray");
    const canvas = document.createElement("canvas");
    const prompts: Array<{ title: string; subtitle: string } | null> = [];
    const stage = new TacsimStage3D(canvas, {
      onSendCommand: () => {},
      onPhysicalLocationChanged: () => {},
      onFocusedPromptChanged: (p) => prompts.push(p),
      onInteractableTriggered: () => {},
      isUiBlockingInput: () => false,
    });
    const marenActor = {
      id: "#42",
      name: "Maren Halvorsen",
      isNpc: true,
      isMale: false,
      shortDesc: "Port Watch Orientation Steward",
    };

    stage.syncWorldState({
      stationPack: ceres,
      shipTemplate: dray,
      isAboardShip: false,
      serverStationRoomEntry: "room.c03_arrivals",
      serverShipCompartment: null,
      roomRevision: 1,
      ambient: decodeTacsimRoomAmbient(null),
      visibleActors: [marenActor],
      actorSpeech: {},
      actorComposing: {},
      roomMarkers: [
        {
          npcId: "#42",
          npcName: "Maren Halvorsen",
          marker: "available",
          symbol: "!",
          questKey: "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll",
          questTitle: "First Papers at the Registry",
        },
      ],
      stowageCargoKg: 0,
      holotankData: null,
    });

    expect(stage.getSpawnedActorQuestMarkerForTest("#42")?.marker).toBe("available");
    expect(stage.getSpawnedActorInteractableForTest("#42")?.title).toBe(
      "[!] Maren Halvorsen — Quest Available",
    );
    expect(stage.getSpawnedActorInteractableForTest("#42")?.npcMeta?.questMarker?.symbol).toBe("!");

    // Transition marker to turn_in and verify prompt updates in-place
    stage.syncWorldState({
      stationPack: ceres,
      shipTemplate: dray,
      isAboardShip: false,
      serverStationRoomEntry: "room.c03_arrivals",
      serverShipCompartment: null,
      roomRevision: 1,
      ambient: decodeTacsimRoomAmbient(null),
      visibleActors: [marenActor],
      actorSpeech: {},
      actorComposing: {},
      roomMarkers: [
        {
          npcId: "#42",
          npcName: "Maren Halvorsen",
          marker: "turn_in",
          symbol: "?",
          questKey: "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll",
          questTitle: "First Papers at the Registry",
        },
      ],
      stowageCargoKg: 0,
      holotankData: null,
    });
    expect(stage.getSpawnedActorQuestMarkerForTest("#42")?.marker).toBe("turn_in");
    expect(stage.getSpawnedActorInteractableForTest("#42")?.title).toBe(
      "[?] Maren Halvorsen — Turn In Quest",
    );

    // Clearing markers reverts NPC prompt cleanly
    stage.syncWorldState({
      stationPack: ceres,
      shipTemplate: dray,
      isAboardShip: false,
      serverStationRoomEntry: "room.c03_arrivals",
      serverShipCompartment: null,
      roomRevision: 1,
      ambient: decodeTacsimRoomAmbient(null),
      visibleActors: [marenActor],
      actorSpeech: {},
      actorComposing: {},
      roomMarkers: [],
      stowageCargoKg: 0,
      holotankData: null,
    });
    expect(stage.getSpawnedActorQuestMarkerForTest("#42")).toBeNull();
    expect(stage.getSpawnedActorInteractableForTest("#42")?.title).toBe("Talk to Maren Halvorsen");
    stage.dispose();

    // 3. Component check: TacsimPortModals NPC dialog exposes [?] Open Quest Dialog handoff for in-progress quest NPC
    const app = new GameApp({
      url: "ws://test/ws",
      webSocketFactory: (u) => new FakeSocket(u),
      random: () => 1,
    });
    app.tacsim.observeRoom({
      id: "#10",
      name: "Registry & Co-op Desk",
      desc: [{ text: "Registry desk." }],
      exits: [],
      contents: [],
      ambient: {
        atlas: { pack: "neighborhood.ceres_freeport", entry: "room.c05_registry" },
      },
    } as RoomView);
    app.quests.applyFrame({
      room_id: "#10",
      room_name: "Registry",
      room_markers: [
        {
          npc_id: "#55",
          npc_name: "Ilse Oduya-Brandt",
          marker: "in_progress",
          symbol: "?",
          quest_key: "q.prov",
          quest_title: "Stock the Locker and Tanks",
          category: "guided",
          quest_count: 1,
        },
      ],
      active: [
        {
          key: "q.prov",
          title: "Stock the Locker and Tanks",
          giver_id: "#55",
          giver_name: "Ilse Oduya-Brandt",
          turn_in_id: "#55",
          turn_in_name: "Ilse Oduya-Brandt",
          checkpoints: [],
        },
      ],
      available: [],
      completed: [],
    });
    app.tacsim.openNpcModal({
      id: "#55",
      name: "Ilse Oduya-Brandt",
      bio: "Registry Clerk",
      isNpc: true,
    });
    render(TacsimSceneDeck, { props: { app, onCmd: vi.fn() } });
    await tick();

    const questBtn = screen.getByRole("button", { name: /\[\?\] Open Quest Dialog/i });
    await fireEvent.click(questBtn);
    await tick();
    expect(app.tacsim.activeModal).toBeNull();
    expect(app.quests.dialogOpen).toBe(true);
    expect(app.quests.dialogNpcId).toBe("#55");
  });

  it("highlights quest trade listings, sets exact quest lot qty, highlights sailing port, and shows quarry checklist", async () => {
    const app = new GameApp({
      url: "ws://test/ws",
      webSocketFactory: (u) => new FakeSocket(u),
      random: () => 1,
    });
    app.tacsim.observeRoom({
      id: "#17",
      name: "Exchange Hall",
      desc: [{ text: "Exchange floor." }],
      exits: [],
      contents: [],
      ambient: {
        atlas: { pack: "neighborhood.ceres_freeport", entry: "room.c07_exchange" },
      },
    } as RoomView);
    app.tacsim.observePortTrade({
      enabled: true,
      venue: "ceres",
      port_name: "Ceres Exchange Hall",
      balance_minor: 500_000,
      balance_text: "5,000.00cr",
      listings: [
        {
          id: 201,
          side: "ask",
          good: "tacsim:metals",
          good_label: "Metals",
          available_qty: 20,
          unit_price_minor: 1500,
          price_text: "15.00cr",
          vendor_name: "Freeport Smelters",
          funded: true,
        },
        {
          id: 202,
          side: "ask",
          good: "tacsim:provisions",
          good_label: "Provisions",
          available_qty: 30,
          unit_price_minor: 900,
          price_text: "9.00cr",
          vendor_name: "Co-op Chandlery",
          funded: true,
        },
      ],
    });
    app.quests.applyFrame({
      schema: 1,
      room_id: "#17",
      room_name: "Exchange Hall",
      room_markers: [],
      available: [],
      completed: [],
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery",
          title: "Metals Down the Corridor",
          checkpoints: [
            {
              id: "deliver_metals",
              title: "Buy 5 tonnes of metals at Ceres and deliver them to Stickney Yards",
              instructions: "Buy 5 metals, run +ship/plan stickney, fly to Stickney Yards, and run +market/sell.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });

    app.tacsim.selectedQtyPreset = 25;
    app.tacsim.activeModal = "trade";
    const deck = render(TacsimSceneDeck, { props: { app, onCmd: vi.fn() } });
    await tick();

    // 1. Trade banner + 1-click Set Quest Qty (5)
    expect(screen.getByRole("status", { name: /active quest trade guidance/i }).textContent).toContain("METALS");
    const setQtyBtn = screen.getByRole("button", { name: /Set Quest Qty \(5\)/i });
    await fireEvent.click(setQtyBtn);
    await tick();
    expect(app.tacsim.selectedQtyPreset).toBe(5);

    // 2. Sailing tab shows Stickney Yards destination banner + 1-click Select Quest Port
    app.tacsim.selectedDestination = "luna";
    app.tacsim.activeModal = "sailing";
    await tick();
    const selPortBtn = screen.getByRole("button", { name: /Select Quest Port \(stickney\)/i });
    await fireEvent.click(selPortBtn);
    await tick();
    expect(app.tacsim.selectedDestination).toBe("stickney");
    deck.unmount();

    // 3. QuarryScreen renders guided mining checklist for Step 6 and switches wash to carbon
    app.quests.applyFrame({
      schema: 1,
      room_id: "#30",
      room_name: "Freeport Commons",
      room_markers: [],
      available: [],
      completed: [],
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
          title: "Scan, Sound, and Mine a Landing Site",
          checkpoints: [
            { id: "scan_landing_site", title: "Scan site", instructions: "Run +tv/1 scan go.", checked: true, live_ready: true },
            { id: "sound_mineral_pocket", title: "Sound pocket", instructions: "Run +tv/1 sound go.", checked: false, live_ready: false },
            { id: "mine_carbon_ore", title: "Mine 1t carbon_ore", instructions: "Run +tv/1 mine go.", checked: false, live_ready: false },
          ],
        },
      ],
    });
    render(QuarryScreen, { props: { app } });
    await tick();
    const pill = screen.getByRole("status", { name: /quest mining checklist/i });
    expect(pill.textContent).toContain("[✓] Scan Site");
    expect(pill.textContent).toContain("Mine carbon_ore");
    const washBtn = screen.getByRole("button", { name: /CARBON wash/i });
    await fireEvent.click(washBtn);
    expect(app.quarry.family).toBe("carbon");
  });

  it("opens quest dialog directly from Ctrl+K command palette entries", () => {
    const app = new GameApp({
      url: "ws://test/ws",
      webSocketFactory: (u) => new FakeSocket(u),
      random: () => 1,
    });
    app.quests.applyFrame({
      schema: 1,
      room_id: "#10",
      room_name: "Port Watch Arrivals",
      room_markers: [
        {
          npc_id: "#42",
          npc_name: "Maren Halvorsen",
          marker: "available",
          symbol: "!",
          quest_key: "q.enroll",
          quest_title: "First Papers at the Registry",
          category: "guided",
          quest_count: 1,
        },
      ],
      available: [
        {
          key: "q.enroll",
          title: "First Papers at the Registry",
          giver_id: "#42",
          giver_name: "Maren Halvorsen",
          turn_in_id: "#55",
          turn_in_name: "Ilse",
          accept_cmd: "+quest/accept #42=tok",
          checkpoints: [],
        },
      ],
      active: [],
      completed: [],
    });

    app.openPalette();
    app.paletteQuery("First Papers");
    const idx = app.paletteEntries.findIndex((e) => e.questAction?.key === "q.enroll");
    expect(idx).toBeGreaterThanOrEqual(0);
    app.paletteSelected = idx;
    app.palettePick(false);
    expect(app.screens.paletteOpen).toBe(false);
    expect(app.quests.dialogOpen).toBe(true);
    expect(app.quests.dialogNpcId).toBe("#42");
  });

  it("renders a 3D golden quest wayfinder trail in TacsimStage3D and guided quest summary in SheetPanel", async () => {
    const app = new GameApp({
      url: "ws://test/ws",
      webSocketFactory: (u) => new FakeSocket(u),
      random: () => 1,
    });
    const pack = app.tacsim.stationPack;
    const startRoom = "room.c03_arrivals";
    let destRoom: string | null = null;
    for (const roomId of pack.rooms.keys()) {
      if (roomId !== startRoom && findStationExitPath(pack, startRoom, roomId).length > 0) {
        destRoom = roomId;
        break;
      }
    }
    expect(destRoom).not.toBeNull();

    const route = computeQuestWayfinderPoints(pack, startRoom, destRoom, 0, 0);
    expect(route).not.toBeNull();
    expect(route!.points.length).toBeGreaterThanOrEqual(2);
    expect(computeQuestWayfinderPoints(pack, startRoom, startRoom, 0, 0)).toBeNull();

    const canvas = document.createElement("canvas");
    const stage = new TacsimStage3D(canvas, {
      onSendCommand: vi.fn(),
      onInteractableTriggered: vi.fn(),
      onFocusedPromptChanged: vi.fn(),
      onPhysicalLocationChanged: vi.fn(),
      isUiBlockingInput: () => false,
    });
    try {
      const baseSync = {
        stationPack: pack,
        shipTemplate: app.tacsim.shipTemplate,
        isAboardShip: false,
        serverStationRoomEntry: startRoom,
        serverShipCompartment: null,
        ambient: app.tacsim.ambient,
        visibleActors: [],
        actorSpeech: {},
        actorComposing: {},
        roomMarkers: [],
        stowageCargoKg: 0,
        holotankData: null,
      };
      stage.syncWorldState({ ...baseSync, roomRevision: 1, questRoomEntry: destRoom });
      const activeTrail = stage.getQuestWayfinderForTest();
      expect(activeTrail.targetRoom).toBe(destRoom);
      expect(activeTrail.hopCount).toBeGreaterThanOrEqual(1);
      expect(activeTrail.childCount).toBe(3);

      // Arriving at the target room clears the 3D wayfinder cleanly
      stage.syncWorldState({
        ...baseSync,
        roomRevision: 2,
        serverStationRoomEntry: destRoom,
        questRoomEntry: destRoom,
      });
      expect(stage.getQuestWayfinderForTest().childCount).toBe(0);
    } finally {
      stage.dispose();
    }

    // SheetPanel shows live guided quest progress and opens Quest Log on click
    expect(
      app.sheet.applyKeyframe({
        target: "#1",
        schema: { ruleset: "tacsim", version: 1, sections: [] },
        approved: { state: "approved", approved: true },
        data: {},
      }),
    ).toBe(true);
    app.quests.applyFrame({
      schema: 1,
      room_id: "#10",
      room_name: "Ceres Concourse",
      room_markers: [],
      available: [],
      active: [
        {
          key: "q.provisions",
          title: "Stock the Locker and Tanks",
          next_step: "Buy at least 5 propellant",
          checkpoints: [
            { id: "prov", title: "Buy 10 provisions", checked: true, live_ready: true },
            { id: "prop", title: "Buy 5 propellant", checked: false, live_ready: false },
          ],
        },
      ],
      completed: [{ key: "q.enroll", title: "First Papers" }],
    });
    const rendered = render(SheetPanel, { props: { app } });
    await tick();
    const card = screen.getByLabelText("guided quest progress");
    expect(card.textContent).toContain("Stock the Locker and Tanks");
    expect(card.textContent).toContain("(1/2)");
    expect(card.textContent).toContain("Completed: 1");
    await fireEvent.click(screen.getByRole("button", { name: "Open Quest Log" }));
    expect(app.quests.logOpen).toBe(true);
    rendered.unmount();
  });

  it("computes multi-hop station quest exit breadcrumbs and renders in-flight quest destination HUD chip", async () => {
    const pack = buildFallbackFreeport();
    const rooms = [...pack.rooms.keys()];
    expect(rooms.length).toBeGreaterThanOrEqual(3);
    const startRoom = rooms[0]!;
    const destRoom = rooms[rooms.length - 1]!;
    const crumb = computeNextQuestExitBreadcrumb(pack, startRoom, destRoom);
    expect(crumb).not.toBeNull();
    expect(crumb!.targetRoomEntry).toBe(destRoom);
    expect(crumb!.hops).toBeGreaterThanOrEqual(1);
    expect(crumb!.nextExitCommand.toLowerCase().startsWith("goto ")).toBe(true);
    expect(computeNextQuestExitBreadcrumb(pack, destRoom, destRoom)).toBeNull();

    // Mounted TacsimSceneDeck renders golden [QUEST] Dest chip aboard ship & at Bridge Helm
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
          panels: [PORT_TRADE_FEED, TACSIM_CONTENT_FEED],
        },
      } as OutputEvent,
      0,
    );
    app.tacsim.view3dOpen = true;
    app.tacsim.isAboardShip = true;
    app.tacsim.seatedAtBridgeHelm = true;
    app.tacsim.ambient = {
      ...app.tacsim.ambient,
      hullName: "M/V Starling",
      docked: false,
    };
    app.quests.applyFrame({
      schema: 1,
      room_id: "#50",
      room_name: "M/V Starling - Bridge",
      room_markers: [],
      available: [],
      completed: [],
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery",
          title: "Run Metals to Stickney Yards",
          next_step: "Sail to Stickney Yards (+ship/plan stickney)",
          checkpoints: [
            {
              id: "sail_stickney",
              title: "Sail to Stickney Yards",
              instructions: "Run +ship/plan stickney and depart.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });

    const sent: string[] = [];
    const deck = render(TacsimSceneDeck, {
      props: { app, onCmd: (c: string) => sent.push(c) },
    });
    await tick();
    const destChips = screen.getAllByRole("button", { name: /\[QUEST\]\s+Dest: Stickney Yards/i });
    expect(destChips.length).toBeGreaterThanOrEqual(2); // port-bar + helm visor bar
    await fireEvent.click(destChips[0]!);
    expect(app.tacsim.selectedDestination).toBe("stickney");
    expect(sent).toContain("+ship/plan stickney");
    deck.unmount();
  });

  it("routes cross-port trade quests to local Exchange first when needsOriginExchangeFirst is true, then flips to Berth Gallery", () => {
    const ceres = buildFallbackFreeport();
    const toExchange = computeNextQuestExitBreadcrumb(
      ceres,
      "room.c05_registry",
      "room.p07_receiving",
      true,
    );
    expect(toExchange).not.toBeNull();
    expect(toExchange!.targetRoomEntry).toBe("room.c07_exchange");

    // Already at Exchange Hall while procuring -> null (stay here to buy)
    expect(
      computeNextQuestExitBreadcrumb(ceres, "room.c07_exchange", "room.p07_receiving", true),
    ).toBeNull();

    // Once cargo is loaded (needsOriginExchangeFirst = false) -> routes to Berth Gallery to board
    const toBerth = computeNextQuestExitBreadcrumb(
      ceres,
      "room.c07_exchange",
      "room.p07_receiving",
      false,
    );
    expect(toBerth).not.toBeNull();
    expect(toBerth!.targetRoomEntry).toBe("room.c01_berth_gallery");
    expect(toBerth!.targetRoomName).toContain("Board Ship");
  });

  it("routes Step 6 surface expedition quests from Rosalind's Exchange Hall to Berth Gallery -> Board Ship", () => {
    const ceres = buildFallbackFreeport();
    // Even standing right in room.c07_exchange (Rosalind's turn-in room), needsShipExpedition routes to Berth Gallery!
    const toBerth = computeNextQuestExitBreadcrumb(
      ceres,
      "room.c07_exchange",
      "room.c07_exchange",
      false,
      true,
    );
    expect(toBerth).not.toBeNull();
    expect(toBerth!.targetRoomEntry).toBe("room.c01_berth_gallery");
    expect(toBerth!.targetRoomName).toContain("Board Ship");

    // At Berth Gallery with needsShipExpedition -> +ship/board
    const board = computeNextQuestExitBreadcrumb(
      ceres,
      "room.c01_berth_gallery",
      "room.c07_exchange",
      false,
      true,
    );
    expect(board?.nextExitCommand).toBe("+ship/board");
    expect(board?.nextRoomName).toBe("Board Ship");
  });

  it("Bridge Flight Visor displays between-steps Next Guided destination chip and Step 6 Ground Chart shortcut", async () => {
    let ws: FakeSocket | null = null;
    const app = new GameApp({
      url: "ws://test/ws",
      webSocketFactory: (u) => (ws = new FakeSocket(u)),
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
          panels: [PORT_TRADE_FEED, TACSIM_CONTENT_FEED],
        },
      } as OutputEvent,
      0,
    );
    app.tacsim.view3dOpen = true;
    app.tacsim.isAboardShip = true;
    app.tacsim.seatedAtBridgeHelm = true;
    app.tacsim.ambient = {
      ...app.tacsim.ambient,
      docked: true,
      port: "Ceres Freeport",
      settledPad: false,
    };
    app.quests.applyFrame({
      schema: 1,
      completed: [
        { key: "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll", title: "S1" },
        { key: "tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel", title: "S2" },
        { key: "tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery", title: "S3" },
      ],
      active: [],
      available: [],
    });

    const sent: string[] = [];
    const deck = render(TacsimSceneDeck, {
      props: { app, onCmd: (c: string) => sent.push(c) },
    });
    await tick();
    const destChips = screen.getAllByRole("button", { name: /\[QUEST\]\s+Dest: Stickney Yards/i });
    expect(destChips.length).toBeGreaterThanOrEqual(2);
    await fireEvent.click(destChips[0]!);
    expect(sent).toContain("+ship/plan stickney");

    // Step 6 surface mining active -> visor shows Ground Chart (CARBON) shortcut
    app.quests.applyFrame({
      schema: 1,
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
          title: "Scan, Sound, and Mine a Landing Site",
          checkpoints: [
            { id: "scan_landing_site", title: "Surface scan", instructions: "+tv/1 scan go", checked: false, live_ready: false },
            { id: "mine_carbon_ore", title: "Mine 1t carbon_ore", instructions: "+tv/1 mine go", checked: false, live_ready: false },
          ],
        },
      ],
      available: [],
      completed: [],
    });
    await tick();
    const mineChips = screen.getAllByRole("button", { name: /\[QUEST\]\s+Mine carbon_ore · Ground Chart/i });
    expect(mineChips.length).toBeGreaterThanOrEqual(1);
    await fireEvent.click(mineChips[0]!);
    expect(app.quarry.mode).toBe("ground");
    expect(app.quarry.family).toBe("carbon");
    expect(sent).toContain("+chart/ground");
    deck.unmount();
  });
});

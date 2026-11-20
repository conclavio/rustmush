// @vitest-environment jsdom
//
// Unit & component integration tests for Track 6 (Doc 48 Economy, 24-Week Price
// History Sparklines, Open Jobs & Crew Board, Works, Shipyard, Bay & Lockers).

import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { describe, expect, it } from "vitest";

import Game from "../../components/Game.svelte";
import { GameApp } from "../game.svelte";
import type { Envelope, OutputEvent } from "../proto";
import type { WebSocketLike } from "../ws";
import {
  formatEscortPostCommand,
  formatHaulPostCommand,
  formatJobCommand,
  formatMarketDepthCommand,
  formatMarketHistoryCommand,
  formatSurveyAcceptCommand,
  HISTORY_GOODS,
  isValidHullDbref,
  parseMarketHistoryLines,
  SIM_UNIT_CHASSIS,
  SIMULATOR_CHASSIS_IDS,
  SURVEY_OFFERS,
  WORKS_RECIPES,
} from "./economy";
import { PORT_TRADE_FEED, TACSIM_CONTENT_FEED } from "./trade";

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

describe("Track 6 Doc 48 Economy, Jobs, Shipyards & Parts Market UI", () => {
  it("parses canonical 24-week +market/history series into sparklines and ranges", () => {
    const parsed = parseMarketHistoryLines([
      "House prices of Volatiles here, by IC week, newest first (4):",
      "  this IC week: ask 120.00 cr, bid 100.00 cr",
      "  last IC week: ask 110.00 cr, bid 92.00 cr",
      "  2 IC weeks ago: ask 100.00 cr, bid 84.00 cr",
      "  3 IC weeks ago: ask 95.00 cr, bid none",
    ]);

    expect(parsed.goodLabel).toBe("Volatiles");
    expect(parsed.rowsNewestFirst).toHaveLength(4);
    expect(parsed.latestAsk).toBe("120.00 cr");
    expect(parsed.latestBid).toBe("100.00 cr");
    expect(parsed.askRange).toBe("95.00 – 120.00");
    expect(parsed.bidRange).toBe("84.00 – 100.00");
    expect(parsed.askSparkline).toHaveLength(4);
    expect(parsed.bidSparkline).toContain("·");

    expect(formatMarketHistoryCommand("metals")).toBe("+market/history tacsim:metals");
    expect(formatMarketDepthCommand("tacsim:volatiles")).toBe("+market/depth tacsim:volatiles");
    expect(formatHaulPostCommand(12, "metals", "stickney", 340, true)).toBe(
      "+haul/post 12 metals to=stickney fee=340 go",
    );
    // C-21 & 48e L8 (clients:G4): +escort/post requires a real `#<u32>` hull dbref, rejecting hull names / `"ship"` / null
    expect(isValidHullDbref("#99")).toBe(true);
    expect(isValidHullDbref("Salt Moth")).toBe(false);
    expect(isValidHullDbref("ship")).toBe(false);
    expect(formatEscortPostCommand("Salt Moth", "stickney", 500, true)).toBeNull();
    expect(formatEscortPostCommand("ship", "stickney", 500, true)).toBeNull();
    expect(formatEscortPostCommand(null, "stickney", 500, true)).toBeNull();
    expect(formatEscortPostCommand("#99", "stickney", 500, true)).toBe(
      "+escort/post #99 to=stickney fee=500 go",
    );
    expect(formatEscortPostCommand("#812", "stickney", 500, true)).toBe(
      "+escort/post #812 to=stickney fee=500 go",
    );
    // H-42: Survey acceptance emits real survey offer ID + `go`
    expect(SURVEY_OFFERS.map((s) => s.id)).toEqual(["ahuna-a", "kerwan-f", "juling-k", "occator-p"]);
    expect(formatSurveyAcceptCommand("ahuna-a", true)).toBe("+survey/accept ahuna-a go");
    expect(formatSurveyAcceptCommand("kerwan-f", false)).toBe("+survey/accept kerwan-f");
    // H-41: Simulator chassis list only includes server-valid SIM UNIT IDs
    expect(SIMULATOR_CHASSIS_IDS.map((u) => u.id)).toEqual([
      "line_tank",
      "line_tank_laser",
      "heavy_mk1",
      "biped_mk1",
      "biped_mk2",
      "scout_mk1",
    ]);
  });

  it("builds job lines only on a typed id, with a separate survey confirm (48e L8, clients:G6)", () => {
    expect(formatJobCommand("+survey/accept", "", false)).toBeNull();
    expect(formatJobCommand("+survey/accept", "  ", true)).toBeNull();
    expect(formatJobCommand("+survey/accept", "12", false)).toBe("+survey/accept 12");
    expect(formatJobCommand("+survey/accept", " 12 ", true)).toBe("+survey/accept 12 go");
    expect(formatJobCommand("+haul/accept", "7", true)).toBe("+haul/accept 7 go");
  });

  it("offers every works recipe and only real goods (48e L8, clients:G11)", () => {
    const content = resolve(__dirname, "../../../../crates/tacsim/content");
    const recipes = JSON.parse(readFileSync(resolve(content, "industry/recipes-v2.json"), "utf-8")) as {
      entries: Record<string, { kind: string; def: { label: string } }>;
    };
    const ids = Object.keys(recipes.entries).filter((k) => recipes.entries[k]!.kind === "recipe");
    expect(WORKS_RECIPES.map((r) => r.id).sort()).toEqual(ids.sort());
    for (const r of WORKS_RECIPES) expect(r.label).toBe(recipes.entries[r.id]!.def.label);

    const goods = JSON.parse(readFileSync(resolve(content, "market/goods.json"), "utf-8")) as {
      goods: Array<{ id: string }>;
    };
    const goodIds = new Set(goods.goods.map((g) => g.id));
    for (const g of HISTORY_GOODS) expect(goodIds.has(g.code), g.code).toBe(true);
  });

  it("offers only simulator chassis that exist as unit files (48e L8, clients:G8)", () => {
    const unitsDir = resolve(__dirname, "../../../../crates/tacsim/content/units");
    for (const u of SIM_UNIT_CHASSIS) {
      expect(existsSync(resolve(unitsDir, `${u.id}.json`)), u.id).toBe(true);
    }
  });

  it("renders 24-Week History (H-39), Jobs & Crew (C-21/H-42), Works & Yard, Simulator (H-41), and unfocused Esc (W3D-M7)", async () => {
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

    render(Game, { props: { app } });
    await tick();

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
            tacsim: { own_hull: "#99", own_hull_name: "Salt Moth", own_hull_docked_here: true },
          },
        },
      } as OutputEvent,
      1,
    );
    await tick();

    await fireEvent.click(screen.getByRole("button", { name: "Port Exchange" }));
    await tick();

    // Preserves single `"command input"` invariant across all Doc 48 inputs!
    expect(screen.getAllByRole("textbox", { name: "command input" })).toHaveLength(1);

    // 1. H-39: Switch to 24-Week History tab & feed a full 25-line (header + 24 IC weeks) server table
    await fireEvent.click(screen.getByRole("button", { name: "24-Week History" }));
    await tick();
    expect(socket.sent.some((raw) => raw.includes("+market/history tacsim:volatiles"))).toBe(true);

    app.tacsim.observeSys("House prices of Volatiles here, by IC week, newest first (24):");
    app.tacsim.observeSys("  this IC week: ask 124.00 cr, bid 104.00 cr");
    app.tacsim.observeSys("  last IC week: ask 123.00 cr, bid 103.00 cr");
    for (let w = 2; w < 24; w++) {
      const ask = (124 - w).toFixed(2);
      const bid = (104 - w).toFixed(2);
      app.tacsim.observeSys(`  ${w} IC weeks ago: ask ${ask} cr, bid ${bid} cr`);
    }
    await tick();

    // Header + all 24 IC weeks survive inside both marketHistoryLines (clients:G5) and modalLog!
    const parsedHistory = parseMarketHistoryLines(app.tacsim.marketHistoryLines);
    expect(parsedHistory.goodLabel).toBe("Volatiles");
    expect(parsedHistory.rowsNewestFirst).toHaveLength(24);
    expect(parseMarketHistoryLines(app.tacsim.modalLog).rowsNewestFirst).toHaveLength(24);
    expect(screen.getByText(/101\.00 – 124\.00/)).toBeTruthy();

    // 2. C-21, H-42 & G6: Switch to Jobs & Crew tab & verify Crew Terms, disabled empty haul, Survey preview+confirm, and Escort `#99`
    await fireEvent.click(screen.getByRole("button", { name: "Jobs & Crew" }));
    await tick();
    expect(socket.sent.some((raw) => raw.includes("+market/jobs"))).toBe(true);

    expect((screen.getByRole("button", { name: "Accept Haul" }) as HTMLButtonElement).disabled).toBe(
      true,
    );
    await fireEvent.click(screen.getByRole("button", { name: "Crew Terms (+sortie/crew)" }));
    expect(socket.sent.some((raw) => raw.includes("+sortie/crew"))).toBe(true);

    await fireEvent.click(screen.getByRole("button", { name: "Preview Survey" }));
    expect(socket.sent.some((raw) => raw.includes("+survey/accept ahuna-a\""))).toBe(true);
    await fireEvent.click(screen.getByRole("button", { name: "Accept Survey" }));
    expect(socket.sent.some((raw) => raw.includes("+survey/accept ahuna-a go"))).toBe(true);

    await fireEvent.click(screen.getByRole("button", { name: /Post Escort \(#99/ }));
    expect(socket.sent.some((raw) => raw.includes("+escort/post #99 to=stickney fee=250 go"))).toBe(true);

    // 3. Switch to Works & Yard tab & verify Works / Yard / Rearm dispatch with `#99`
    await fireEvent.click(screen.getByRole("button", { name: "Works & Yard" }));
    await tick();
    expect(socket.sent.some((raw) => raw.includes("+works"))).toBe(true);

    await fireEvent.click(screen.getByRole("button", { name: "Rearm Magazines (+bay/order =rearm)" }));
    expect(socket.sent.some((raw) => raw.includes("+bay/order #99=rearm"))).toBe(true);

    // 4. W3D-M7: Pressing Escape on window closes the modal even when body (not modal) has focus
    (document.body as HTMLElement).focus();
    await fireEvent.keyDown(window, { key: "Escape" });
    await tick();
    expect(app.tacsim.activeModal).toBeNull();

    // 5. H-41: Simulator Console modal emits valid server chassis IDs (`scout_mk1`, `heavy_mk1`), never bogus IDs
    app.tacsim.openModal("sim");
    await tick();
    await fireEvent.click(screen.getByRole("button", { name: "Scout Mk1" }));
    await fireEvent.click(screen.getByRole("button", { name: "Heavy Mk1" }));
    expect(socket.sent.some((raw) => raw.includes("SIM UNIT scout_mk1"))).toBe(true);
    expect(socket.sent.some((raw) => raw.includes("SIM UNIT heavy_mk1"))).toBe(true);
    expect(screen.queryByRole("button", { name: /^scout$/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /missile_carrier/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /heavy_mech/i })).toBeNull();
  });
});

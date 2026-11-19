// @vitest-environment jsdom
//
// Unit & component integration tests for Track 6 (Doc 48 Economy, 24-Week Price
// History Sparklines, Open Jobs & Crew Board, Works, Shipyard, Bay & Lockers).

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
  formatMarketDepthCommand,
  formatMarketHistoryCommand,
  parseMarketHistoryLines,
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
    expect(formatEscortPostCommand("Salt Moth", "stickney", 500, true)).toBe(
      "+escort/post Salt Moth to=stickney fee=500 go",
    );
  });

  it("renders 24-Week History, Jobs & Crew, and Works & Yard tabs inside Port Exchange", async () => {
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
            tacsim: { own_hull_name: "Salt Moth", own_hull_docked_here: true },
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

    // 1. Switch to 24-Week History tab & feed real `mushd` history lines
    await fireEvent.click(screen.getByRole("button", { name: "24-Week History" }));
    await tick();
    expect(socket.sent.some((raw) => raw.includes("+market/history tacsim:volatiles"))).toBe(true);

    app.tacsim.observeSys("House prices of Volatiles here, by IC week, newest first (2):");
    app.tacsim.observeSys("  this IC week: ask 115.00 cr, bid 96.00 cr");
    app.tacsim.observeSys("  last IC week: ask 105.00 cr, bid 88.00 cr");
    await tick();

    expect(screen.getByText(/105\.00 – 115\.00/)).toBeTruthy();

    // 2. Switch to Jobs & Crew tab & verify Crew Terms + Haul dispatch
    await fireEvent.click(screen.getByRole("button", { name: "Jobs & Crew" }));
    await tick();
    expect(socket.sent.some((raw) => raw.includes("+market/jobs"))).toBe(true);

    await fireEvent.click(screen.getByRole("button", { name: "Crew Terms (+sortie/crew)" }));
    expect(socket.sent.some((raw) => raw.includes("+sortie/crew"))).toBe(true);

    // 3. Switch to Works & Yard tab & verify Works / Yard / Rearm dispatch
    await fireEvent.click(screen.getByRole("button", { name: "Works & Yard" }));
    await tick();
    expect(socket.sent.some((raw) => raw.includes("+works"))).toBe(true);

    await fireEvent.click(screen.getByRole("button", { name: "Rearm Magazines (+bay/order =rearm)" }));
    expect(socket.sent.some((raw) => raw.includes("+bay/order Salt Moth=rearm"))).toBe(true);
  });
});

// The Corridor port store's content sync and history capture (48e L8).
//
// clients:G1 — the server answers a `tacsim.content` pack only to
// `{ op: "pack", id }`; anything else is the manifest again. The store asks for
// each pack once, so a manifest repeat asks for nothing and a missing pack is
// never re-asked.
// clients:G5 — a `+market/history` reply is a heading and up to 24 week rows;
// the store keeps the whole reply, however many other lines the log shows.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { TacsimPortStore } from "./tacsim.svelte";
import { parseMarketHistoryLines } from "../tacsim/economy";

type Call = [string, Record<string, unknown> | undefined];

function storeWithCalls(): { store: TacsimPortStore; calls: Call[] } {
  const calls: Call[] = [];
  const store = new TacsimPortStore({
    hasPanel: () => true,
    subscribePanel: (feed, params) => {
      calls.push([feed, params]);
      return true;
    },
    sendCommand: () => {},
    selfCharacterId: () => null,
  });
  return { store, calls };
}

const GOODS_JSON = readFileSync(
  resolve(__dirname, "../../../../crates/tacsim/content/market/goods.json"),
  "utf-8",
);

describe("tacsim.content pack sync (48e L8, clients:G1)", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("asks each wanted pack once with op pack, and never again", () => {
    const { store, calls } = storeWithCalls();
    const manifest = {
      op: "manifest",
      packs: [
        { id: "neighborhood.ceres_freeport", category: "neighborhood", sha256: "a" },
        { id: "cast.ceres_freeport", category: "cast", sha256: "b" },
        { id: "dray", category: "unit", sha256: "c" },
        { id: "goods", category: "market", sha256: "d" },
        { id: "arena.proving_ground", category: "arena", sha256: "e" },
      ],
    };
    store.observeContentFeed(manifest);
    vi.advanceTimersByTime(2_000);
    expect(calls).toEqual([
      ["tacsim.content", { op: "pack", id: "neighborhood.ceres_freeport" }],
      ["tacsim.content", { op: "pack", id: "cast.ceres_freeport" }],
      ["tacsim.content", { op: "pack", id: "dray" }],
      ["tacsim.content", { op: "pack", id: "goods" }],
    ]);

    // A manifest repeat (what the old `{ pack }` request got back each time)
    // and a pack the server no longer has ask for nothing more.
    calls.length = 0;
    store.observeContentFeed(manifest);
    store.observeContentFeed({ op: "pack", found: false, id: "dray" });
    store.observeContentFeed(manifest);
    vi.advanceTimersByTime(2_000);
    expect(calls).toEqual([]);

    // The pack reply loads.
    store.observeContentFeed({
      op: "pack",
      found: true,
      id: "goods",
      requested_id: "goods",
      category: "market",
      sha256: "d",
      json: GOODS_JSON,
    });
    expect(store.goodsById.get("torpedo")?.label).toBe("space torpedoes, canistered");
  });
});

describe("+market/history capture (48e L8, clients:G5)", () => {
  it("keeps a 24-week reply whole, heading and newest week included", () => {
    const { store } = storeWithCalls();
    store.openModal("trade");
    store.observeSys("You check the board.");
    store.observeSys("House prices of Volatiles here, by IC week, newest first (24):");
    store.observeSys("  this IC week: ask 100.00 cr, bid 90.00 cr");
    store.observeSys("  last IC week: ask 99.00 cr, bid 89.00 cr");
    for (let n = 2; n < 24; n += 1) {
      store.observeSys(`  ${n} IC weeks ago: ask ${(100 - n).toFixed(2)} cr, bid 80.00 cr`);
    }
    store.observeSys("Margit says, \"Anything else?\"");

    const parsed = parseMarketHistoryLines(store.marketHistoryLines);
    expect(parsed.goodLabel).toBe("Volatiles");
    expect(parsed.rowsNewestFirst).toHaveLength(24);
    expect(parsed.latestAsk).toBe("100.00 cr");

    // A new heading starts a fresh reply: an earlier good's rows are gone.
    store.observeSys("House prices of Metals here, by IC week, newest first (1):");
    store.observeSys("  this IC week: ask 210.00 cr, bid 190.00 cr");
    store.observeSys("  last IC week: ask 1.00 cr, bid 1.00 cr");
    const metals = parseMarketHistoryLines(store.marketHistoryLines);
    expect(metals.goodLabel).toBe("Metals");
    expect(metals.rowsNewestFirst.map((r) => r.askText)).toEqual(["210.00 cr"]);
  });
});

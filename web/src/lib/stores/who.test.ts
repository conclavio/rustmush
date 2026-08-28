// WhoStore: idle/conn tick math between refreshes.

import { describe, expect, it } from "vitest";

import type { WhoEntry } from "../proto";
import { WhoStore } from "./who.svelte";

const entry = (over: Partial<WhoEntry> = {}): WhoEntry => ({
  name: "Wizard",
  idle_s: 30,
  conn_s: 600,
  ...over,
});

describe("WhoStore", () => {
  it("ticks idle and conn forward locally from the fetch time", () => {
    const who = new WhoStore();
    who.replace([entry()], 100_000);
    const e = who.entries[0]!;
    expect(who.idleOf(e, 100_000)).toBe(30);
    expect(who.idleOf(e, 145_000)).toBe(75); // +45s elapsed
    expect(who.connOf(e, 145_000)).toBe(645);
  });

  it("a refresh resets the tick baseline", () => {
    const who = new WhoStore();
    who.replace([entry()], 100_000);
    who.replace([entry({ idle_s: 0 })], 200_000); // the player just typed
    const e = who.entries[0]!;
    expect(who.idleOf(e, 200_000)).toBe(0);
    expect(who.idleOf(e, 203_000)).toBe(3);
  });

  it("never ticks backwards on clock skew", () => {
    const who = new WhoStore();
    who.replace([entry()], 100_000);
    expect(who.idleOf(who.entries[0]!, 99_000)).toBe(30);
  });

  it("clear empties the rail", () => {
    const who = new WhoStore();
    who.replace([entry()], 1);
    who.clear();
    expect(who.count).toBe(0);
    expect(who.fetchedAt).toBe(0);
  });
});

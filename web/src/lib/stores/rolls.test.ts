// RollsStore: ring buffer of wod.roll panel pushes.

import { describe, expect, it } from "vitest";

import { RollsStore } from "./rolls.svelte";

function rollData(face: number, who = "#1") {
  return {
    who,
    summary: `rolled ${face}`,
    outcome: {
      difficulty: 6,
      dice: [{ face, is_success: face >= 6, is_one: face === 1, doubled: false }],
      raw_successes: face >= 6 ? 1 : 0,
      ones: face === 1 ? 1 : 0,
      willpower_success: false,
      extra_successes: 0,
      net: face >= 6 ? 1 : 0,
      result: face >= 6 ? { Success: 1 } : "Failure",
    },
  };
}

describe("RollsStore", () => {
  it("push prepends the newest roll and assigns unique rowIds", () => {
    const r = new RollsStore();
    const a = r.push(rollData(8));
    const b = r.push(rollData(9));
    expect(a?.rowId).not.toBe(b?.rowId);
    expect(r.entries[0]?.outcome?.dice[0]?.face).toBe(9); // newest first
    expect(r.entries[1]?.outcome?.dice[0]?.face).toBe(8);
  });

  it("caps the ring at 30 entries", () => {
    const r = new RollsStore();
    for (let i = 0; i < 35; i++) {
      r.push(rollData(6));
    }
    expect(r.entries).toHaveLength(30);
  });

  it("a malformed push is dropped (returns null, list unchanged)", () => {
    const r = new RollsStore();
    r.push(rollData(8));
    const result = r.push({ garbage: true });
    expect(result).toBeNull();
    expect(r.entries).toHaveLength(1);
  });

  // I6-C11: `Owod20Outcome` is ONE ruleset's shape — it needs a
  // `difficulty`, which a Shadowrun pool has no notion of. Requiring it
  // dropped every `sr.roll` frame, so the recent-rolls table and the dice
  // card were permanently empty on a Shadowrun server while the TUI, which
  // renders the summary, was fine.
  it("keeps a roll from another engine, on its summary alone", () => {
    const r = new RollsStore();
    const entry = r.push({
      who: "#1",
      summary: "Nick rolls Agility + Pistols: 3 hits.",
      outcome: { hits: 3, glitch: false, pool: 9 }, // Sr5-shaped: no difficulty
    });

    expect(entry).not.toBeNull();
    expect(r.entries).toHaveLength(1);
    expect(r.entries[0]?.summary).toBe("Nick rolls Agility + Pistols: 3 hits.");
    // The oWoD20 detail is absent, not faked — the dice card is gated on it.
    expect(r.entries[0]?.outcome).toBeNull();
  });

  it("still drops a frame with neither a summary nor an outcome", () => {
    const r = new RollsStore();
    expect(r.push({ who: "#1" })).toBeNull();
    expect(r.push({ who: "#1", summary: "" })).toBeNull();
    expect(r.push({ summary: "no roller" })).toBeNull();
    expect(r.entries).toHaveLength(0);
  });

  it("clear empties the ring", () => {
    const r = new RollsStore();
    r.push(rollData(8));
    r.clear();
    expect(r.entries).toHaveLength(0);
  });
});

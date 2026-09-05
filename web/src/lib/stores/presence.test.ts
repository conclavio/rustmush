// PresenceStore: thinking and composing pills raise, clear, and expire.

import { describe, expect, it } from "vitest";

import { COMPOSING_TTL_S, THINKING_TTL_S } from "../composing";
import { PresenceStore } from "./presence.svelte";

describe("PresenceStore", () => {
  it("thinking pills follow the frame's active flag and expire", () => {
    const p = new PresenceStore();
    p.applyThinking({ npc: "#9", name: "Bartender", active: true }, 1000);
    expect(p.thinking.map((t) => t.name)).toEqual(["Bartender"]);
    p.applyThinking({ npc: "#9", name: "Bartender" }, 2000);
    expect(p.thinking[0]?.at).toBe(2000);
    p.applyThinking({ npc: "#9", name: "Bartender", active: false }, 3000);
    expect(p.thinking).toEqual([]);
    p.applyThinking({ name: "Cook" }, 4000);
    p.prune(4000 + THINKING_TTL_S * 1000 - 1);
    expect(p.thinking.length).toBe(1);
    p.prune(4000 + THINKING_TTL_S * 1000);
    expect(p.thinking).toEqual([]);
    p.applyThinking({ npc: "#1" }, 1);
    expect(p.thinking).toEqual([]);
  });

  it("composing pills clear on inactive and expire", () => {
    const p = new PresenceStore();
    p.applyComposing("Zoe", true, 1000);
    p.applyComposing("Zoe", true, 1500);
    expect(p.composing).toEqual([{ name: "Zoe", at: 1500 }]);
    p.applyComposing("Al", true, 1600);
    p.applyComposing("Zoe", false, 1700);
    expect(p.composing.map((c) => c.name)).toEqual(["Al"]);
    p.prune(1600 + COMPOSING_TTL_S * 1000);
    expect(p.composing).toEqual([]);
    p.clear();
  });
});

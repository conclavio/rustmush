// SessionStore: hello-derived feature/panel gates, incl. the suffix-derived
// sheetFeed/rollFeed (never a hardcoded ruleset id — see their doc comments).

import { describe, expect, it } from "vitest";

import { SessionStore } from "./session.svelte";

describe("SessionStore.sheetFeed / rollFeed", () => {
  it("derives the active ruleset's feeds from the panel manifest by suffix", () => {
    const s = new SessionStore();
    s.applyHello({ server: "rustmush", features: ["wod"], panels: ["comsys", "mail", "wod.sheet", "wod.roll"] });
    expect(s.sheetFeed).toBe("wod.sheet");
    expect(s.rollFeed).toBe("wod.roll");
  });

  it("null when no ruleset panel is registered", () => {
    const s = new SessionStore();
    s.applyHello({ server: "rustmush", features: [], panels: ["comsys", "mail"] });
    expect(s.sheetFeed).toBeNull();
    expect(s.rollFeed).toBeNull();
  });

  it("would follow a differently-named ruleset with no client change", () => {
    const s = new SessionStore();
    s.applyHello({ server: "rustmush", features: ["btech"], panels: ["btech.sheet", "btech.roll"] });
    expect(s.sheetFeed).toBe("btech.sheet");
    expect(s.rollFeed).toBe("btech.roll");
  });

  it("breaks a two-ruleset tie by sorting, not by manifest order (LB-21)", () => {
    // The manifest's order is the server's registration order, and the
    // gateway's bridge and this client walked it differently — so two
    // clients on the same server could disagree about which sheet is "the"
    // sheet. The TUI already sorts; this now matches it, whatever order the
    // manifest arrives in.
    const panels = ["wod.sheet", "wod.roll", "sr.sheet", "sr.roll", "tactics.unit"];
    const s = new SessionStore();
    s.applyHello({ server: "rustmush", features: [], panels });
    expect(s.sheetFeed).toBe("sr.sheet");
    expect(s.rollFeed).toBe("sr.roll");

    const reversed = new SessionStore();
    reversed.applyHello({ server: "rustmush", features: [], panels: [...panels].reverse() });
    expect(reversed.sheetFeed).toBe("sr.sheet");
    expect(reversed.rollFeed).toBe("sr.roll");
    expect(reversed.unitFeed).toBe("tactics.unit");
  });
});

describe("SessionStore.worldgenAvailable", () => {
  it("true when the hello advertises the worldgen feature", () => {
    const s = new SessionStore();
    s.applyHello({ server: "rustmush", features: ["worldgen"], panels: [] });
    expect(s.worldgenAvailable).toBe(true);
  });

  it("true when the hello advertises the worldgen.draft panel, feature absent", () => {
    const s = new SessionStore();
    s.applyHello({ server: "rustmush", features: [], panels: ["worldgen.draft"] });
    expect(s.worldgenAvailable).toBe(true);
  });

  it("false when neither is advertised", () => {
    const s = new SessionStore();
    s.applyHello({ server: "rustmush", features: ["wod"], panels: ["comsys", "mail", "wod.sheet"] });
    expect(s.worldgenAvailable).toBe(false);
  });
});

describe("SessionStore.llmDashboardAvailable", () => {
  it("true when the hello advertises the llm.dashboard panel", () => {
    const s = new SessionStore();
    s.applyHello({ server: "rustmush", features: ["worldgen"], panels: ["llm.dashboard"] });
    expect(s.llmDashboardAvailable).toBe(true);
  });

  it("false when the panel is absent, even with worldgen otherwise available", () => {
    const s = new SessionStore();
    s.applyHello({ server: "rustmush", features: ["worldgen"], panels: ["worldgen.draft"] });
    expect(s.llmDashboardAvailable).toBe(false);
  });
});

describe("SessionStore.assetPack (doc 21 cut G.11)", () => {
  it("keeps the pack the hello names, and drops it when a later hello names none", () => {
    const s = new SessionStore();
    expect(s.assetPack).toBeNull();
    const pack = { version: 1, pack_version: "pack_0123456789abcdef", manifest: "/assets/manifest.json" };
    s.applyHello({ server: "rustmush", features: [], panels: [], asset_pack: pack });
    expect(s.assetPack).toEqual(pack);
    s.applyHello({ server: "rustmush", features: [], panels: [] });
    expect(s.assetPack).toBeNull();
  });
});

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
});

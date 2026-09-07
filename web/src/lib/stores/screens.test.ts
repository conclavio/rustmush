// ScreensStore: the binding-table probe, availability from the manifest,
// feed-registered screens, and the digits shared with the TUI.

import { describe, expect, it } from "vitest";

import {
  BUILTIN_SCREENS,
  ScreensStore,
  UNIT_SCREEN,
  validateScreens,
  type GateInput,
  type ScreenDef,
} from "./screens.svelte";

const full: GateInput = {
  panels: ["comsys", "mail", "wod.sheet", "wod.roll", "legal.actions", "worldgen.draft", "llm.dashboard"],
  features: ["mail", "channels"],
};
const bare: GateInput = { panels: [], features: [] };

describe("validateScreens", () => {
  it("accepts the built-ins and refuses a mutating run, a reserved letter, a global key", () => {
    expect(validateScreens(BUILTIN_SCREENS)).toBeNull();
    const fire: ScreenDef = {
      ...BUILTIN_SCREENS[0]!,
      keys: [{ key: "f", label: "fire", action: { kind: "run", cmd: "+fire a" } }],
    };
    expect(validateScreens([fire])).toContain("not a read-only verb");
    const reserved: ScreenDef = {
      ...BUILTIN_SCREENS[0]!,
      keys: [{ key: "n", label: "new", action: { kind: "prefill", line: "@dig " } }],
    };
    expect(validateScreens([reserved])).toContain("reserved for exits");
    const mailN: ScreenDef = {
      ...BUILTIN_SCREENS[2]!,
      keys: [{ key: "n", label: "new", action: { kind: "prefill", line: "@mail/send " } }],
    };
    expect(validateScreens([mailN])).toBeNull();
    const global: ScreenDef = {
      ...BUILTIN_SCREENS[1]!,
      keys: [{ key: "?", label: "help", action: { kind: "local", action: "refresh" } }],
    };
    expect(validateScreens([global])).toContain("global key");
    expect(validateScreens([BUILTIN_SCREENS[0]!, { ...BUILTIN_SCREENS[1]!, digit: 1 }])).toContain(
      "used twice",
    );
  });
});

describe("ScreensStore", () => {
  it("shares the TUI's digits and gates screens on the manifest", () => {
    const s = new ScreensStore();
    const digits = Object.fromEntries(s.available(full).map((d) => [d.id, d.digit]));
    expect(digits).toEqual({
      scene: 1,
      sheet: 2,
      mail: 3,
      channels: 4,
      rolls: 5,
      actions: 7,
      build: 8,
      llm: 9,
    });
    expect(s.available(bare).map((d) => d.id)).toEqual(["scene"]);
    expect(s.switchDigit(3, bare)).toBe(false);
    expect(s.active).toBe("scene");
    expect(s.switchDigit(3, full)).toBe(true);
    expect(s.active).toBe("mail");
    expect(s.activeDef(bare).id).toBe("scene"); // falls back when the screen is gone
    expect(s.bindingKeys(full)).toEqual(new Set(["r", "s"]));
    expect(s.binding("s", full)?.action).toEqual({ kind: "prefill", line: "@mail/send " });
  });

  it("a feed-registered screen appears only when the manifest carries its feed", () => {
    const s = new ScreensStore();
    const def: ScreenDef = {
      id: "tactics",
      digit: 6,
      label: "tactics",
      keys: [{ key: "t", label: "target", action: { kind: "prefill", line: "+target " } }],
      exitsAreKeys: false,
      gate: () => true,
    };
    expect(s.registerFeedScreen({ suffix: ".unit", def })).toBeNull();
    expect(s.available(full).map((d) => d.id)).not.toContain("tactics");
    const withUnit: GateInput = { ...full, panels: [...full.panels, "tactics.unit"] };
    expect(s.available(withUnit).map((d) => d.id)).toContain("tactics");
    expect(s.switchDigit(6, withUnit)).toBe(true);
    const bad: ScreenDef = { ...def, id: "bad", digit: 6 };
    expect(s.registerFeedScreen({ suffix: ".x", def: bad })).toContain("used twice");
  });

  it("exits are keys on the scene screen only", () => {
    const s = new ScreensStore();
    s.exits = new Map([["n", "North"]]);
    expect(s.exitKeysFor(full).get("n")).toBe("North");
    s.switchTo("mail", full);
    expect(s.exitKeysFor(full).size).toBe(0);
    s.clear();
    expect(s.active).toBe("scene");
    expect(s.exits.size).toBe(0);
  });
});

describe("registerViewScreen (interface plan I9)", () => {
  it("registers a .view screen on a free digit, gated on the feed, idempotently", () => {
    const s = new ScreensStore();
    const withView: GateInput = { panels: [...full.panels, "jobs.view"], features: full.features };
    expect(s.registerViewScreen("jobs.view")).toBeNull();
    const def = s.available(withView).find((d) => d.id === "view:jobs.view");
    expect(def).toBeDefined();
    expect(def?.digit).toBe(6); // the one free web digit
    expect(def?.label).toBe("jobs");
    expect(def?.viewFeed).toBe("jobs.view");
    // Absent from the manifest, absent from the list.
    expect(s.available(full).some((d) => d.id === "view:jobs.view")).toBe(false);
    // Switch to it by its digit.
    expect(s.switchDigit(6, withView)).toBe(true);
    expect(s.active).toBe("view:jobs.view");
    // Idempotent: a second call adds nothing.
    expect(s.registerViewScreen("jobs.view")).toBeNull();
    expect(s.available(withView).filter((d) => d.id === "view:jobs.view")).toHaveLength(1);
  });

  it("refuses a second view screen: 6 is the only free web digit", () => {
    // The built-ins hold 1-5 and 7-9; a first view screen takes 6, and a
    // second has no free digit (a real constraint — a fuller policy is a
    // later question, tracked in the plan's I9 as-built).
    const s = new ScreensStore();
    expect(s.registerViewScreen("jobs.view")).toBeNull();
    expect(s.registerViewScreen("staff.view")).toContain("no free hotkey digit");
  });
});

describe("the tactical unit screen (interface cut I8)", () => {
  it("takes digit 6, refuses a second registration, and closes the digit", () => {
    // D-I8-4: ONE screen hosting both layouts, `t` flipping them. The cost
    // is recorded here rather than discovered in production — on a server
    // publishing both a `.unit` and a `.view` feed, the view screen has
    // nowhere to go.
    const s = new ScreensStore();
    expect(validateScreens([...BUILTIN_SCREENS, UNIT_SCREEN.def])).toBeNull();
    expect(UNIT_SCREEN.suffix).toBe(".unit");
    expect(s.registerFeedScreen(UNIT_SCREEN)).toBeNull();
    const withUnit: GateInput = { ...full, panels: [...full.panels, "tactics.unit"] };
    const def = s.available(withUnit).find((d) => d.id === "unit");
    expect(def?.digit).toBe(6);
    expect(def?.label).toBe("unit");
    expect(def?.keys.map((b) => b.key)).toEqual(["r", "t", "c", "d"]);
    // `@unit/damage` mutates and is staff-gated: a Prefill, never a Run.
    expect(def?.keys.find((b) => b.key === "d")?.action).toEqual({
      kind: "prefill",
      line: "@unit/damage ",
    });
    expect(def?.keys.find((b) => b.key === "t")?.action).toEqual({
      kind: "local",
      action: "toggle",
    });
    // Absent from the manifest, absent from the list.
    expect(s.available(full).some((d) => d.id === "unit")).toBe(false);
    expect(s.switchDigit(6, withUnit)).toBe(true);
    // `registerFeedScreen` is not idempotent — GameApp registers exactly once.
    expect(s.registerFeedScreen(UNIT_SCREEN)).toContain("declared twice");
    expect(s.registerViewScreen("jobs.view")).toContain("no free hotkey digit");
  });
});

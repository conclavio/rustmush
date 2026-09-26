// ScreensStore: the binding-table probe, availability from the manifest,
// feed-registered screens, and the digits shared with the TUI.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import {
  BUILTIN_SCREENS,
  RUN_ALLOWLIST,
  ScreensStore,
  UNIT_SCREEN,
  validateScreens,
  type GateInput,
  type ScreenDef,
} from "./screens.svelte";

const full: GateInput = {
  panels: ["comsys", "mail", "wod.sheet", "wod.roll", "legal.actions", "worldgen.draft", "llm.dashboard"],
  features: ["mail", "channels"],
  cockpitSeated: false,
  pictureOpen: false,
};
const bare: GateInput = { panels: [], features: [], cockpitSeated: false, pictureOpen: false };
/** A server that also publishes the arena's cockpit feed (16b cut Q.5), seen by
 * a pilot a frame has seated (16b cut B.3). */
const arena: GateInput = {
  panels: [...full.panels, "arena.cockpit"],
  features: full.features,
  cockpitSeated: true,
  pictureOpen: false,
};
/** The same server seen by a bystander, or by a pilot after the cleared frame. */
const unseated: GateInput = { ...arena, cockpitSeated: false };

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
    // 16b cut Q.5 opened the range to the cockpit's tenth digit; ten is still
    // not a digit.
    expect(validateScreens([{ ...BUILTIN_SCREENS[0]!, digit: 10 }])).toContain("is not 0-9");
    expect(validateScreens([{ ...BUILTIN_SCREENS[0]!, digit: -1 }])).toContain("is not 0-9");
  });
});

describe("ScreensStore", () => {
  it("shares the TUI's digits and gates screens on the manifest", () => {
    const s = new ScreensStore();
    const digits = Object.fromEntries(s.available(full).map((d) => [d.id, d.digit]));
    // No cockpit here: this manifest carries no `arena.cockpit` (16b cut Q.5).
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

  it("the cockpit takes digit 0, reads LAST in the strip, and is gated on the feed", () => {
    const s = new ScreensStore();
    // Absent from a manifest without the feed — a server with no arena has no
    // cockpit tab at all.
    expect(s.available(full).some((d) => d.id === "cockpit")).toBe(false);
    expect(s.switchDigit(0, full)).toBe(false);
    // Present with it, on 0, and LAST: the strip reads `1 … 9 0`, which is the
    // order of the digits on a keyboard and the order the TUI's tab bar reads.
    const ids = s.available(arena).map((d) => d.id);
    expect(ids[ids.length - 1]).toBe("cockpit");
    expect(s.available(arena).find((d) => d.id === "cockpit")?.digit).toBe(0);
    expect(s.switchDigit(0, arena)).toBe(true);
    expect(s.active).toBe("cockpit");
    // …and the fallback is still the scene screen, never the cockpit. This has
    // to be asked of a gate the cockpit PASSES: under `full` it is filtered out
    // by its own gate before the sort is consulted, so that case says nothing
    // about the order. A viewer whose active screen vanished must not land in
    // a cockpit they are not seated in, which is what a raw `a.digit - b.digit`
    // sort would do.
    s.active = "a screen that went away";
    expect(s.activeDef(arena).id).toBe("scene");
    expect(s.activeDef(full).id).toBe("scene");
    // The twelve keys: the seven read-only verbs run, the five orders only
    // prefill, and no order verb is on the allowlist.
    const keys = s.available(arena).find((d) => d.id === "cockpit")?.keys ?? [];
    expect(keys.filter((b) => b.action.kind === "run").map((b) => b.action)).toEqual([
      { kind: "run", cmd: "view" },
      { kind: "run", cmd: "contacts" },
      { kind: "run", cmd: "status" },
      { kind: "run", cmd: "weaponstatus" },
      { kind: "run", cmd: "range" },
      { kind: "run", cmd: "bearing" },
      { kind: "run", cmd: "scan" },
    ]);
    expect(keys.filter((b) => b.action.kind === "prefill").map((b) => b.key)).toEqual([
      "k",
      "f",
      "h",
      "m",
      "s",
    ]);
    for (const verb of ["startup", "heading", "speed", "lock", "fire"]) {
      expect(RUN_ALLOWLIST).not.toContain(verb);
    }
  });

  it("the cockpit tab needs a seat as well as the feed (16b cut B.3, `T-63`, Q21)", () => {
    const s = new ScreensStore();
    // A bystander on an arena server: the feed is published, no frame has
    // seated them, so there is no tab and the digit is inert — the TUI's rule.
    expect(s.available(unseated).some((d) => d.id === "cockpit")).toBe(false);
    expect(s.switchDigit(0, unseated)).toBe(false);
    // Seated: the tab, on 0, and the digit reaches it.
    expect(s.available(arena).some((d) => d.id === "cockpit")).toBe(true);
    expect(s.switchDigit(0, arena)).toBe(true);
    expect(s.activeDef(arena).id).toBe("cockpit");
    // The seat goes while the pilot stands on the tab: it is no longer drawn,
    // and `activeDef` puts them on the scene.
    expect(s.activeDef(unseated).id).toBe("scene");
    // A seat without the feed is no tab either (a manifest that dropped it).
    expect(s.available({ ...full, cockpitSeated: true }).some((d) => d.id === "cockpit")).toBe(
      false,
    );
  });

  it("the cockpit tab stands for a replay with no seat, and its keys are the replay's (16b cut I.2)", () => {
    const s = new ScreensStore();
    // A replay open on an arena server that publishes the replay feed: the tab
    // with no seat, on 0, and the digit reaches it.
    const replaying: GateInput = {
      ...unseated,
      panels: [...unseated.panels, "arena.replay"],
      pictureOpen: true,
    };
    expect(s.available(replaying).some((d) => d.id === "cockpit")).toBe(true);
    expect(s.switchDigit(0, replaying)).toBe(true);
    expect(s.activeDef(replaying).id).toBe("cockpit");
    // Its binding table is the replay's five, and the cockpit's twelve are not
    // dispatched under it…
    expect(s.activeKeys(replaying).map((b) => `${b.key} ${b.label}`)).toEqual([
      ", prev",
      ". next",
      "u focus",
      "x close",
      "p pull",
    ]);
    expect(s.binding("k", replaying)).toBeNull();
    expect(s.binding(",", replaying)?.action).toEqual({ kind: "local", action: "picturePrev" });
    expect([...s.bindingKeys(replaying)].sort()).toEqual([",", ".", "p", "u", "x"]);
    // …nor the replay's under a seat, where the table is the cockpit's own.
    expect(s.binding(",", arena)).toBeNull();
    expect(s.binding("k", arena)?.label).toBe("lock");
    // No replay feed in the manifest, or no picture: no tab for a bystander.
    expect(s.available({ ...replaying, panels: unseated.panels }).some((d) => d.id === "cockpit")).toBe(
      false,
    );
    expect(s.available({ ...replaying, pictureOpen: false }).some((d) => d.id === "cockpit")).toBe(
      false,
    );
    // The second table is held to every rule the first is: a `run` of `sim`
    // or a mutating verb, a global key, a key bound twice.
    const cockpit = BUILTIN_SCREENS.find((d) => d.id === "cockpit")!;
    const withAlt = (extra: ScreenDef["keys"][number]): ScreenDef => ({
      ...cockpit,
      altKeys: [...(cockpit.altKeys ?? []), extra],
    });
    expect(validateScreens([withAlt({ key: "r", label: "r", action: { kind: "run", cmd: "SIM REPLAY 7 next" } })])).toContain(
      "sim is never a bare key",
    );
    expect(validateScreens([withAlt({ key: "f", label: "f", action: { kind: "run", cmd: "fire BA" } })])).toContain(
      "not a read-only verb",
    );
    expect(validateScreens([withAlt({ key: "g", label: "g", action: { kind: "local", action: "pictureNext" } })])).toContain(
      "global key",
    );
    expect(validateScreens([withAlt({ key: ",", label: "again", action: { kind: "local", action: "picturePrev" } })])).toContain(
      "bound twice",
    );
    expect(validateScreens(BUILTIN_SCREENS)).toBeNull();
    // Only the cockpit has a second table.
    expect(BUILTIN_SCREENS.filter((d) => d.altKeys !== undefined).map((d) => d.id)).toEqual(["cockpit"]);
  });

  it("the cockpit's `p` pulls, right after `m`, and the table still validates (16b cut B.2)", () => {
    // `g` would have been refused: it is the pane-jump key both clients take
    // before any binding. `p` is on neither reserved list, and it sits right
    // after `m speed` so a clipped legend still shows it — the TUI's order too.
    const keys = BUILTIN_SCREENS.find((d) => d.id === "cockpit")?.keys ?? [];
    const at = keys.findIndex((b) => b.key === "m");
    expect(at).toBeGreaterThanOrEqual(0);
    expect(keys[at + 1]).toEqual({
      key: "p",
      label: "pull",
      action: { kind: "local", action: "refresh" },
    });
    expect(keys.filter((b) => b.action.kind === "local").map((b) => b.key)).toEqual(["p"]);
    expect(validateScreens(BUILTIN_SCREENS)).toBeNull();
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
    const withView: GateInput = {
      panels: [...full.panels, "jobs.view"],
      features: full.features,
      cockpitSeated: false,
      pictureOpen: false,
    };
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

// LB-16: `RUN_ALLOWLIST` is two hand-kept copies — this one and
// `mush-net-ssh/src/tui/screen.rs` — with nothing that crossed between them.
// Both are now pinned to one committed fixture, this side in a vitest and
// the TUI side in a Rust test, so a verb added to one and not the other
// fails a build rather than diverging in silence.
describe("the run allowlist", () => {
  const fixture: string[] = JSON.parse(
    readFileSync(resolve(process.cwd(), "src/lib/run-allowlist.json"), "utf8"),
  ) as string[];

  it("matches the committed fixture the TUI is pinned to", () => {
    const sorted = [...RUN_ALLOWLIST].sort();
    expect(new Set(sorted).size).toBe(RUN_ALLOWLIST.length); // no verb twice
    expect(sorted).toEqual(fixture);
  });

  it("carries no writer: a bare key mutates nothing", () => {
    // `@mail <n>` marks the message read and `+orders` writes work orders;
    // both rode this list until LB-16. `@mail` was unreachable from the TUI
    // anyway (it refuses any `@` Run), which is how a stale entry survives
    // unnoticed on a hand-kept list.
    expect(fixture).not.toContain("@mail");
    expect(fixture).not.toContain("+orders");
    expect(fixture.filter((v) => v.startsWith("@"))).toEqual([]);
  });

  it("lists every verb a screen actually binds, and reserves the rest", () => {
    const bound = [...BUILTIN_SCREENS, UNIT_SCREEN.def]
      .flatMap((s) => s.keys)
      .flatMap((b) => (b.action.kind === "run" ? [b.action.cmd.split(/\s+/)[0] ?? ""] : []));
    // 16b cut Q.5 bound the cockpit's seven read-only verbs, which 16b cut Q.4
    // had already put on the list for the TUI's copy of the same screen.
    expect(bound).toEqual([
      "look",
      "inventory",
      "+roll/last",
      "view",
      "contacts",
      "status",
      "weaponstatus",
      "range",
      "bearing",
      "scan",
      "+contacts",
    ]);
    for (const verb of bound) {
      expect(fixture).toContain(verb);
    }
  });
});

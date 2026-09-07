// UnitStore (interface cut I8): keyframe hydration and the honest no-unit
// frame, the journal ring and its four outcomes, the pane helpers, the pane
// order, and the coalescing storm guard. Fixtures are the frozen contract's
// F1 (keyframe) and F3 (journal delta) verbatim.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it, vi } from "vitest";

import {
  afterAction,
  beforeAfterRows,
  capabilityRows,
  cellsUnder,
  componentGroups,
  ELIDED_KIND,
  installSection,
  journalEntryText,
  journalRows,
  journalTableRows,
  JOURNAL_RING,
  JOURNAL_ROWS_PER_DELTA,
  mobilityLine,
  parseUnitView,
  truncatedFooter,
  UNIT_PANE_TITLES,
  UNIT_PLACEHOLDERS,
  UNIT_RESYNC_MS,
  unitCells,
  unitPaneOrder,
  UnitStore,
  type JournalRow,
  type UnitView,
} from "./unit.svelte";

/** Contract fixture F1 — the keyframe. */
const F1 = {
  object: { id: "#7", name: "Atlas" },
  schema: "tacsim:unit:atlas@1#" + "0".repeat(64),
  revision: 3,
  state: "active",
  record: null,
  mobility: {
    mode: "ground.biped",
    switching: null,
    modes: [
      {
        id: "ground.biped",
        locomotion: "legged",
        mediums: ["ground"],
        tags: [],
        nominal_speed_cms: 640,
        transition_s: 0,
      },
    ],
  },
  sections: [
    {
      id: "center_torso",
      condition: "ok",
      state: "crit",
      facets: [{ id: "rear", cells: ["armor"] }],
      slots: ["torso_hardpoint"],
    },
  ],
  cells: [
    { addr: "unit/heat", label: "heat", current: 20, min: 0, max: 30, state: "ok" },
    {
      addr: "section/center_torso/structure",
      label: "structure",
      current: 8,
      min: 0,
      max: 8,
      state: "ok",
    },
    {
      addr: "facet/center_torso/rear/armor",
      label: "armor",
      current: 1,
      min: 0,
      max: 6,
      state: "crit",
    },
    { addr: "component/gauss/ammo", label: "ammo", current: 9, min: 0, max: 12, state: "ok" },
  ],
  components: [
    {
      id: "gauss",
      definition: "tacsim:component:gauss@1#" + "0".repeat(64),
      install: "slot/center_torso/torso_hardpoint",
      condition: "ok",
      state: "ok",
      asset: false,
    },
  ],
  capabilities: [{ id: "tactics.mobility", source: null }],
  counts: { sections: 1, cells: 4, components: 1, capabilities: 1 },
  truncated: false,
};

/** Contract fixture F3 — the projected journal delta. */
const F3 = {
  object: { id: "#7", name: "Atlas" },
  from_revision: 3,
  to_revision: 4,
  now: 1757000000,
  cause: "gauss",
  entries: [
    { kind: "cell", addr: "facet/center_torso/rear/armor", from: 6, to: 1 },
    { kind: "section", section: "center_torso", from: "ok", to: "damaged" },
    { kind: "component", component: "gauss", from: "ok", to: "destroyed" },
    { kind: "asset", component: "gauss", attached: false },
    { kind: "capability_granted", id: "tactics.mobility", source: "gauss", replaced: false },
    { kind: "capability_revoked", id: "tactics.mobility", source: "gauss", was_present: true },
  ],
};

/** A delta of `n` entries, at revision `rev`, for object `id`. */
function delta(rev: number, n: number, id = "#7"): unknown {
  return {
    object: { id, name: "Atlas" },
    from_revision: rev - 1,
    to_revision: rev,
    now: 1757000000,
    cause: null,
    entries: Array.from({ length: n }, (_, i) => ({ kind: "cell", addr: `unit/c${i}`, from: i, to: 0 })),
  };
}

describe("parseUnitView", () => {
  it("reads every field of the contract keyframe", () => {
    const v = parseUnitView(F1)!;
    expect(v.object).toEqual({ id: "#7", name: "Atlas" });
    expect(v.schema).toBe(F1.schema);
    expect(v.revision).toBe(3);
    expect(v.state).toBe("active");
    expect(v.record).toBeNull();
    expect(v.truncated).toBe(false);
    expect(v.counts).toEqual({ sections: 1, cells: 4, components: 1, capabilities: 1 });
    expect(v.mobility?.mode).toBe("ground.biped");
    expect(v.mobility?.switching).toBeNull();
    expect(v.mobility?.modes).toEqual([
      {
        id: "ground.biped",
        locomotion: "legged",
        mediums: ["ground"],
        tags: [],
        nominal_speed_cms: 640,
        transition_s: 0,
      },
    ]);
    expect(v.sections).toEqual([
      {
        id: "center_torso",
        condition: "ok",
        state: "crit",
        facets: [{ id: "rear", cells: ["armor"] }],
        slots: ["torso_hardpoint"],
      },
    ]);
    expect(v.cells.map((c) => c.addr)).toEqual([
      "unit/heat",
      "section/center_torso/structure",
      "facet/center_torso/rear/armor",
      "component/gauss/ammo",
    ]);
    expect(v.cells[2]).toEqual({
      addr: "facet/center_torso/rear/armor",
      label: "armor",
      current: 1,
      min: 0,
      max: 6,
      state: "crit",
    });
    expect(v.components).toEqual([
      {
        id: "gauss",
        definition: F1.components[0]!.definition,
        install: "slot/center_torso/torso_hardpoint",
        condition: "ok",
        state: "ok",
        asset: false,
      },
    ]);
    expect(v.capabilities).toEqual([{ id: "tactics.mobility", source: null }]);
  });

  it("is null on garbage and never throws; a held frame keeps the same key set", () => {
    expect(parseUnitView(null)).toBeNull();
    expect(parseUnitView("nope")).toBeNull();
    expect(parseUnitView(42)).toBeNull();
    expect(parseUnitView([F1])).toBeNull();
    expect(parseUnitView({})).toBeNull();
    expect(parseUnitView({ object: { id: "#7" } })).toBeNull(); // no revision
    expect(parseUnitView({ object: {}, revision: 1 })).toBeNull(); // no object id
    const held = parseUnitView({
      object: { id: "#7", name: "Atlas" },
      schema: "",
      revision: 0,
      state: "frozen",
      record: "unit:7",
      mobility: null,
      sections: [],
      cells: [],
      components: [],
      capabilities: [],
      counts: {},
      truncated: false,
    })!;
    expect(held.state).toBe("frozen");
    expect(held.record).toBe("unit:7");
    expect(held.mobility).toBeNull();
    expect(held.sections).toEqual([]);
  });

  it("drops malformed members rather than the whole frame", () => {
    const v = parseUnitView({
      ...F1,
      cells: [...F1.cells, { addr: "unit/x" }, "nope"],
      sections: [...F1.sections, { condition: "ok" }],
      components: [...F1.components, 7],
      capabilities: [...F1.capabilities, {}],
      counts: { cells: 4, bad: "no" },
    })!;
    expect(v.cells).toHaveLength(4);
    expect(v.sections).toHaveLength(1);
    expect(v.components).toHaveLength(1);
    expect(v.capabilities).toHaveLength(1);
    expect(v.counts).toEqual({ cells: 4 });
  });
});

describe("journalRows", () => {
  it("tags every row with the delta header", () => {
    const rows = journalRows(F3)!;
    expect(rows).toHaveLength(6);
    expect(rows[0]).toEqual({
      object: "#7",
      rev: 4,
      cause: "gauss",
      at: 1757000000,
      entry: F3.entries[0],
    });
    expect(rows.every((r) => r.rev === 4 && r.object === "#7")).toBe(true);
  });

  it("is null on anything that is not a delta", () => {
    expect(journalRows(null)).toBeNull();
    expect(journalRows("nope")).toBeNull();
    expect(journalRows({ ...F3, object: {} })).toBeNull();
    expect(journalRows({ ...F3, to_revision: "4" })).toBeNull();
    expect(journalRows({ ...F3, entries: "no" })).toBeNull();
  });

  it("caps one delta at 49 rows plus an elided row", () => {
    expect(journalRows(delta(4, JOURNAL_ROWS_PER_DELTA))).toHaveLength(JOURNAL_ROWS_PER_DELTA);
    const rows = journalRows(delta(4, 60))!;
    expect(rows).toHaveLength(JOURNAL_ROWS_PER_DELTA);
    expect(rows[JOURNAL_ROWS_PER_DELTA - 1]!.entry).toEqual({ kind: ELIDED_KIND, omitted: 11 });
    expect(journalEntryText(rows[JOURNAL_ROWS_PER_DELTA - 1]!.entry)).toBe("… 11 more entries");
  });
});

describe("UnitStore keyframes", () => {
  it("starts empty", () => {
    const s = new UnitStore();
    expect(s.loaded).toBe(false);
    expect(s.view).toBeNull();
    expect(s.revision).toBe(0);
    expect(s.unavailable).toBeNull();
    expect(s.layout).toBe("unit");
    expect(s.objectId).toBeNull();
  });

  it("applyKeyframe replaces, seeds the watermark and clears the guards", () => {
    const s = new UnitStore();
    s.markUnavailable("You have no station on that unit.");
    s.resyncPending = true;
    expect(s.applyKeyframe(F1)).toBe(true);
    expect(s.loaded).toBe(true);
    expect(s.revision).toBe(3);
    expect(s.objectId).toBe("#7");
    expect(s.unavailable).toBeNull();
    expect(s.resyncPending).toBe(false);
  });

  it("applyKeyframe(null) is the honest no-unit state, not an error", () => {
    const s = new UnitStore();
    s.applyKeyframe(F1);
    s.resyncPending = true;
    expect(s.applyKeyframe(null)).toBe(true);
    expect(s.view).toBeNull();
    expect(s.loaded).toBe(false);
    expect(s.revision).toBe(0);
    expect(s.unavailable).toBeNull();
    expect(s.resyncPending).toBe(false);
  });

  it("applyKeyframe on garbage leaves the held view untouched", () => {
    const s = new UnitStore();
    s.applyKeyframe(F1);
    expect(s.applyKeyframe({ nonsense: true })).toBe(false);
    expect(s.revision).toBe(3);
    expect(s.objectId).toBe("#7");
  });

  it("a keyframe for a different object clears the ring", () => {
    const s = new UnitStore();
    s.applyKeyframe(F1);
    s.applyJournal(F3);
    expect(s.journal).toHaveLength(6);
    s.applyKeyframe({ ...F1, revision: 9 }); // same object: the ring survives
    expect(s.journal).toHaveLength(6);
    s.applyKeyframe({ ...F1, object: { id: "#9", name: "Other" } });
    expect(s.journal).toHaveLength(0);
    expect(s.objectId).toBe("#9");
  });

  it("an equal revision replaces rather than dropping", () => {
    const s = new UnitStore();
    s.applyKeyframe(F1);
    s.applyKeyframe({ ...F1, state: "frozen", record: "unit:7" });
    expect(s.view?.state).toBe("frozen");
    expect(s.revision).toBe(3);
  });
});

describe("UnitStore journal outcomes", () => {
  it("resyncs when nothing is held, and still rings the rows", () => {
    const s = new UnitStore();
    expect(s.applyJournal(F3)).toBe("resync");
    expect(s.journal).toHaveLength(6);
  });

  it("resyncs when the delta runs ahead of the watermark", () => {
    const s = new UnitStore();
    s.applyKeyframe(F1); // revision 3
    expect(s.applyJournal(F3)).toBe("resync"); // to_revision 4 > 3
  });

  it("is stale at or behind the watermark", () => {
    const s = new UnitStore();
    s.applyKeyframe(F1);
    expect(s.applyJournal({ ...F3, to_revision: 3 })).toBe("stale");
    expect(s.applyJournal({ ...F3, to_revision: 1 })).toBe("stale");
    expect(s.revision).toBe(3);
    expect(s.journal).toHaveLength(12); // both still ringed
  });

  it("is foreign for another object, and rings it tagged", () => {
    const s = new UnitStore();
    s.applyKeyframe(F1);
    expect(s.applyJournal({ ...F3, object: { id: "#9", name: "Other" } })).toBe("foreign");
    expect(s.journal[0]?.object).toBe("#9");
  });

  it("is malformed on a payload that is not a delta, and rings nothing", () => {
    const s = new UnitStore();
    s.applyKeyframe(F1);
    expect(s.applyJournal(null)).toBe("malformed");
    expect(s.applyJournal("nope")).toBe("malformed");
    expect(s.applyJournal({ ...F3, entries: 4 })).toBe("malformed");
    expect(s.journal).toHaveLength(0);
  });

  it("the ring caps at JOURNAL_RING, newest first", () => {
    const s = new UnitStore();
    s.applyKeyframe(F1);
    for (let rev = 4; rev <= 8; rev++) {
      s.applyJournal(delta(rev, JOURNAL_ROWS_PER_DELTA));
    }
    expect(s.journal).toHaveLength(JOURNAL_RING);
    expect(s.journal[0]?.rev).toBe(8); // the newest delta is at the front
  });
});

describe("UnitStore guards and reset", () => {
  it("markUnavailable blanks the panel and releases the storm guard", () => {
    const s = new UnitStore();
    s.resyncPending = true;
    s.markUnavailable("You have no station on that unit.");
    expect(s.unavailable).toBe("You have no station on that unit.");
    expect(s.loaded).toBe(false);
    expect(s.view).toBeNull();
    expect(s.resyncPending).toBe(false);
  });

  it("toggleLayout flips both ways", () => {
    const s = new UnitStore();
    s.toggleLayout();
    expect(s.layout).toBe("damage");
    s.toggleLayout();
    expect(s.layout).toBe("unit");
  });

  it("clear resets everything, the storm guard included", () => {
    const s = new UnitStore();
    s.applyKeyframe(F1);
    s.applyJournal(F3);
    s.toggleLayout();
    s.resyncPending = true;
    s.clear();
    expect(s.view).toBeNull();
    expect(s.revision).toBe(0);
    expect(s.journal).toEqual([]);
    expect(s.loaded).toBe(false);
    expect(s.resyncPending).toBe(false);
    expect(s.unavailable).toBeNull();
    expect(s.layout).toBe("unit");
  });

  it("coalesces a burst of deltas into ONE refresh after UNIT_RESYNC_MS", () => {
    // The timer itself lives in GameApp (`#scheduleUnitResync`), which has
    // no constructible test harness (no jsdom, no socket seam on the app) —
    // so this drives the exact rule that file implements over the real
    // store: arm once while the guard is down, never while it is up. The
    // wiring in game.svelte.ts is pinned by the source probe below.
    vi.useFakeTimers();
    try {
      const s = new UnitStore();
      s.applyKeyframe(F1);
      let refreshes = 0;
      let timer: ReturnType<typeof setTimeout> | null = null;
      const onDelta = (raw: unknown): void => {
        if (s.applyJournal(raw) === "resync" && !s.resyncPending) {
          s.resyncPending = true;
          if (timer === null) {
            timer = setTimeout(() => {
              timer = null;
              refreshes += 1;
            }, UNIT_RESYNC_MS);
          }
        }
      };
      onDelta(delta(4, 1));
      onDelta(delta(5, 1));
      onDelta(delta(6, 1));
      expect(refreshes).toBe(0);
      vi.advanceTimersByTime(UNIT_RESYNC_MS - 1);
      expect(refreshes).toBe(0);
      vi.advanceTimersByTime(1);
      expect(refreshes).toBe(1);
      // The reply releases the guard; the next burst may ask again.
      s.resyncPending = false;
      onDelta(delta(7, 1));
      vi.advanceTimersByTime(UNIT_RESYNC_MS);
      expect(refreshes).toBe(2);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("pane helpers", () => {
  const view = parseUnitView(F1)!;

  it("cellsUnder scopes to a section (its own and its facets') or a component", () => {
    expect(cellsUnder(view, "section", "center_torso").map((c) => c.addr)).toEqual([
      "section/center_torso/structure",
      "facet/center_torso/rear/armor",
    ]);
    expect(cellsUnder(view, "component", "gauss").map((c) => c.addr)).toEqual([
      "component/gauss/ammo",
    ]);
    expect(cellsUnder(view, "section", "nope")).toEqual([]);
    expect(unitCells(view.cells).map((c) => c.addr)).toEqual(["unit/heat"]);
  });

  it("afterAction keeps the newest revision of the held object only", () => {
    const ring: JournalRow[] = [
      { object: "#7", rev: 5, cause: null, at: 0, entry: { kind: "cell", addr: "a", from: 2, to: 1 } },
      { object: "#9", rev: 9, cause: null, at: 0, entry: { kind: "cell", addr: "b", from: 2, to: 1 } },
      { object: "#7", rev: 4, cause: null, at: 0, entry: { kind: "cell", addr: "c", from: 2, to: 1 } },
    ];
    expect(afterAction(view, ring).map((r) => r.entry.addr)).toEqual(["a"]);
    expect(afterAction(null, ring)).toEqual([]);
    expect(afterAction(view, [])).toEqual([]);
  });

  it("beforeAfterRows carries both sides of a cell, a section and a component", () => {
    const s = new UnitStore();
    s.applyKeyframe(F1);
    s.applyJournal(F3);
    expect(beforeAfterRows(afterAction(s.view, s.journal))).toEqual([
      { addr: "facet/center_torso/rear/armor", before: "6", after: "1" },
      { addr: "section/center_torso", before: "ok", after: "damaged" },
      { addr: "component/gauss", before: "ok", after: "destroyed" },
    ]);
  });

  it("journal rows read as text, unknown kinds opaquely, foreign rows named", () => {
    const rows = journalRows(F3)!;
    expect(rows.map((r) => journalEntryText(r.entry))).toEqual([
      "cell facet/center_torso/rear/armor 6→1",
      "section center_torso ok→damaged",
      "component gauss ok→destroyed",
      "asset gauss detached",
      "capability_granted tactics.mobility gauss",
      "capability_revoked tactics.mobility gauss",
    ]);
    expect(journalEntryText({ kind: "future_kind", payload: { deep: 1 } })).toBe("future_kind");
    expect(journalEntryText({})).toBe("");
    const table = journalTableRows(rows, "#7");
    expect(table[0]).toEqual({
      rev: "4",
      cause: "gauss",
      entry: "cell facet/center_torso/rear/armor 6→1",
    });
    expect(journalTableRows(rows, "#9")[0]?.entry).toContain("#7");
    expect(journalTableRows([{ ...rows[0]!, cause: null }], "#7")[0]?.cause).toBe("—");
  });

  it("the mobility line reads a mode, a switch in progress, or none", () => {
    expect(mobilityLine(view.mobility)).toBe("mobility ground.biped");
    expect(mobilityLine(null)).toBe("mobility none");
    expect(mobilityLine({ mode: null, switching: null, modes: [] })).toBe("mobility none");
    expect(
      mobilityLine({
        mode: "a",
        switching: { from: "a", to: "b", ready_at: 12 },
        modes: [],
      }),
    ).toBe("mobility a → b (ready 12)");
  });

  it("a truncated frame footers what it could not carry", () => {
    expect(truncatedFooter(view)).toBeNull();
    const cut: UnitView = { ...view, truncated: true, counts: { cells: 9, components: 4 } };
    expect(truncatedFooter(cut)).toBe("view truncated: 5 cells, 3 components not shown");
  });

  it("components group by install section, the object's own group first", () => {
    const groups = componentGroups({
      ...view,
      components: [
        ...view.components,
        {
          id: "core",
          definition: "",
          install: "unit",
          condition: "ok",
          state: "warn",
          asset: true,
        },
      ],
      capabilities: [
        { id: "tactics.mobility", source: null },
        { id: "tactics.action", source: "gauss" },
      ],
    });
    expect(groups.map((g) => g.id)).toEqual(["unit", "center_torso"]);
    expect(groups[0]!.rows).toEqual([
      { key: "core", value: "ok", state: "warn", detail: "asset" },
      { key: "tactics.mobility", value: "", state: "", detail: null },
    ]);
    expect(groups[1]!.rows).toEqual([
      { key: "gauss", value: "ok", state: "ok", detail: null },
      { key: "tactics.action", value: "", state: "", detail: null },
    ]);
    expect(capabilityRows(view)).toEqual([
      { key: "tactics.mobility", value: "—", state: "", detail: null },
    ]);
  });

  it("installSection reads the flat install path", () => {
    expect(installSection("unit")).toBeNull();
    expect(installSection("section/center_torso")).toBe("center_torso");
    expect(installSection("slot/center_torso/torso_hardpoint")).toBe("center_torso");
    expect(installSection("")).toBeNull();
  });
});

describe("unitPaneOrder", () => {
  const placeholders = UNIT_PLACEHOLDERS.map((p) => p.id);

  it("puts the panes with no producer last in both layouts", () => {
    for (const layout of ["unit", "damage"] as const) {
      const panes = unitPaneOrder(layout, false);
      expect(panes.slice(-placeholders.length)).toEqual(placeholders);
      expect(panes.slice(0, -placeholders.length).length).toBeGreaterThan(0);
    }
    expect(unitPaneOrder("unit", false).slice(0, 4)).toEqual([
      "sections",
      "plant",
      "components",
      "capabilities",
    ]);
    expect(unitPaneOrder("damage", false).slice(0, 3)).toEqual([
      "sectionmap",
      "afteraction",
      "journal",
    ]);
  });

  it("a live contacts table takes its own placeholder's slot, still ahead of the rest", () => {
    const panes = unitPaneOrder("unit", true);
    expect(panes.filter((p) => p === "contacts")).toHaveLength(1);
    const rest = placeholders.filter((p) => p !== "contacts");
    expect(panes.slice(-rest.length)).toEqual(rest);
    expect(panes.indexOf("contacts")).toBeLessThan(panes.indexOf(rest[0]!));
  });

  it("every pane has a title", () => {
    for (const layout of ["unit", "damage"] as const) {
      for (const pane of unitPaneOrder(layout, true)) {
        const title = UNIT_PANE_TITLES[pane] ?? UNIT_PLACEHOLDERS.find((p) => p.id === pane)?.title;
        expect(title, pane).toBeTruthy();
      }
    }
    expect(UNIT_PLACEHOLDERS.map((p) => p.title)).toEqual([
      "grid: awaiting a geometry projection (plan §9)",
      "weapons: awaiting a weapons-cycle projection (plan §9)",
      "comms: awaiting a station comms projection (plan §9)",
      "contacts: awaiting tactics.contacts (plan §9)",
    ]);
  });
});

describe("the unit panel leaks no definition body", () => {
  // The client analogue of the server's projection-cleanliness assertion
  // (and of genericview.test.ts's offers probe): a frame whose identity
  // strings are replaced by a marker, and whose every level carries a stray
  // `rules` object, must surface neither through the parser nor through any
  // pane helper.
  const MARK = "zzdefinitionbodyzz";
  const dirty = {
    ...F1,
    schema: MARK,
    rules: { hidden: MARK },
    mobility: {
      ...F1.mobility,
      rules: { hidden: MARK },
      modes: [{ ...F1.mobility.modes[0], rules: { hidden: MARK } }],
    },
    sections: [{ ...F1.sections[0], rules: { hidden: MARK } }],
    cells: F1.cells.map((c) => ({ ...c, rules: { hidden: MARK } })),
    components: [{ ...F1.components[0], definition: MARK, rules: { hidden: MARK } }],
  };

  it("the parser carries no key it does not name", () => {
    const v = parseUnitView(dirty)!;
    // `schema`/`definition` are identities the contract keeps; blank them
    // and nothing else may carry the marker or a rules body.
    const scrubbed = JSON.stringify({
      ...v,
      schema: "",
      components: v.components.map((c) => ({ ...c, definition: "" })),
    });
    expect(scrubbed).not.toContain(MARK);
    expect(scrubbed.toLowerCase()).not.toContain("rules");
  });

  it("no pane helper renders an identity or a rules body", () => {
    const v = parseUnitView(dirty)!;
    const s = new UnitStore();
    s.applyKeyframe(dirty);
    s.applyJournal(F3);
    const rendered = [
      ...journalTableRows(s.journal, v.object.id).map((r) => `${r.rev} ${r.cause} ${r.entry}`),
      ...beforeAfterRows(afterAction(v, s.journal)).map((r) => `${r.addr} ${r.before} ${r.after}`),
      ...componentGroups(v).flatMap((g) => [g.label, ...g.rows.map((r) => `${r.key} ${r.value} ${r.detail ?? ""}`)]),
      ...capabilityRows(v).map((r) => `${r.key} ${r.value}`),
      ...v.sections.map((sec) => `${sec.id} ${sec.condition} ${sec.state}`),
      ...v.cells.map((c) => `${c.label} ${c.addr} ${c.state}`),
      mobilityLine(v.mobility),
      truncatedFooter(v) ?? "",
      ...Object.values(UNIT_PANE_TITLES),
      ...UNIT_PLACEHOLDERS.map((p) => p.title),
    ].join("\n");
    expect(rendered).not.toContain(MARK);
    expect(rendered.toLowerCase()).not.toContain("rules");
  });
});

describe("the coalesced unit resync is wired in GameApp", () => {
  // No component/app harness exists (vitest runs `environment: "node"` and
  // GameApp owns its WsClient), so the wiring the store cannot see is
  // pinned by source probe — the convention vitalsboard.test.ts already
  // uses on this tree.
  const src = readFileSync(resolve(process.cwd(), "src/lib/game.svelte.ts"), "utf8");

  it("arms one timeout at UNIT_RESYNC_MS and never doubles it", () => {
    expect(src).toContain("#scheduleUnitResync");
    expect(src).toMatch(/#unitResyncTimer\s*!==\s*null\)\s*\{\s*return;/);
    expect(src).toMatch(/setTimeout\([\s\S]{0,200}?\},\s*UNIT_RESYNC_MS\)/);
    expect(src).toMatch(/applyJournal\(d\.data\)\s*===\s*"resync"\s*&&\s*!this\.unit\.resyncPending/);
  });

  it("disarms the timer and clears the store on both reset seams", () => {
    // `#resetGameState` and `onFullRehydrate` — a timer left armed across a
    // disconnect would latch the storm guard forever.
    expect(src.match(/this\.#clearUnitResync\(\);/g) ?? []).toHaveLength(2);
    expect(src.match(/this\.unit\.clear\(\);/g) ?? []).toHaveLength(2);
    expect(src).toMatch(/pending\?\.kind === "unit"[\s\S]{0,400}?resyncPending = false/);
  });

  it("gives the layout toggle its own arm and the screen its refresh arm", () => {
    expect(src).toMatch(/case "toggle":[\s\S]{0,400}?toggleLayout\(\)/);
    expect(src).toMatch(/case "unit":\s*this\.refreshUnit\(\);\s*this\.refreshContacts\(\);/);
  });
});

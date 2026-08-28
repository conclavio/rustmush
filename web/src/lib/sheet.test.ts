// Sheet schema/doc pure-function tests: dots/pip rendering, health marks, a
// splat section (Disciplines — a Map<Dots>), and staff_only omission.

import { describe, expect, it } from "vitest";

import {
  approvalKind,
  asStringList,
  barPct,
  capitalize,
  deepestLevel,
  isPipRange,
  layoutKind,
  listItems,
  mapEntries,
  parseApproval,
  parseSheetPayload,
  parseSheetSchema,
  pipRow,
  trackCells,
  trackMarkSymbol,
  visibleFields,
  type SheetSchema,
  type SheetSection,
} from "./sheet";

describe("pipRow / isPipRange / barPct", () => {
  it("fills `value` pips out of `max`, ignoring `min`", () => {
    expect(pipRow(3, 5)).toEqual([true, true, true, false, false]);
    expect(pipRow(0, 5)).toEqual([false, false, false, false, false]);
    expect(pipRow(5, 5)).toEqual([true, true, true, true, true]);
  });

  it("clamps out-of-range values", () => {
    expect(pipRow(-2, 5)).toEqual([false, false, false, false, false]);
    expect(pipRow(99, 5)).toEqual([true, true, true, true, true]);
    expect(pipRow(3, 0)).toEqual([]);
  });

  it("truncates fractional inputs", () => {
    expect(pipRow(2.9, 5)).toEqual([true, true, false, false, false]);
  });

  it("small-span Number fields (Willpower, Rage, Gnosis) are pip range", () => {
    expect(isPipRange(1, 10)).toBe(true); // willpower
    expect(isPipRange(0, 10)).toBe(true); // rage / gnosis
    expect(isPipRange(4, 15)).toBe(true); // generation
  });

  it("wide-span Number fields (Blood Pool, Quintessence, XP) are not", () => {
    expect(isPipRange(0, 50)).toBe(false); // blood pool
    expect(isPipRange(0, 100)).toBe(false); // quintessence / paradox
    expect(isPipRange(0, 1_000_000)).toBe(false); // xp
  });

  it("barPct is a clamped 0..100 proportion", () => {
    expect(barPct(25, 0, 50)).toBe(50);
    expect(barPct(-5, 0, 50)).toBe(0);
    expect(barPct(999, 0, 50)).toBe(100);
    expect(barPct(1, 1, 1)).toBe(0); // zero span never divides by zero
  });
});

describe("track marks (Health)", () => {
  it("maps the classic marks to their glyphs", () => {
    expect(trackMarkSymbol("bashing")).toBe("/");
    expect(trackMarkSymbol("lethal")).toBe("X");
    expect(trackMarkSymbol("aggravated")).toBe("*");
    expect(trackMarkSymbol(null)).toBe("");
  });

  it("falls back to an initial for an unknown mark (future resource tracks)", () => {
    expect(trackMarkSymbol("blood")).toBe("B");
  });

  it("trackCells validates the wire array defensively", () => {
    expect(trackCells(["bashing", null, "lethal"])).toEqual(["bashing", null, "lethal"]);
    expect(trackCells([1, "lethal", {}])).toEqual([null, "lethal", null]);
    expect(trackCells("not an array")).toEqual([]);
    expect(trackCells(null)).toEqual([]);
  });

  it("deepestLevel maps filled-slot count to the wound level (SheetStore::wound_penalty parity)", () => {
    const levels = [
      { id: "bruised", label: "Bruised", penalty: 0 },
      { id: "hurt", label: "Hurt", penalty: -1 },
      { id: "injured", label: "Injured", penalty: -1 },
      { id: "wounded", label: "Wounded", penalty: -2 },
    ];
    expect(deepestLevel([null, null, null, null], levels)).toBeNull();
    expect(deepestLevel(["bashing", null, null, null], levels)?.id).toBe("bruised");
    expect(deepestLevel(["bashing", "bashing", "lethal", "lethal"], levels)?.id).toBe("wounded");
    // A track longer than its level list clamps to the last level.
    const marks = ["bashing", "bashing", "bashing", "bashing", "bashing"];
    expect(deepestLevel(marks, levels)?.id).toBe("wounded");
  });

  it("empty level list -> no current level even with damage", () => {
    expect(deepestLevel(["bashing"], [])).toBeNull();
  });
});

describe("layoutKind", () => {
  it("normalizes the SectionLayout union", () => {
    expect(layoutKind("list")).toBe("list");
    expect(layoutKind("tracks")).toBe("tracks");
    expect(layoutKind("map")).toBe("map");
    expect(layoutKind({ grid: { rows: 3, cols: 3 } })).toBe("grid");
  });
});

describe("maps / lists", () => {
  const field = {
    id: "disciplines",
    label: "Disciplines",
    ty: { kind: "map", value: { kind: "dots", min: 0, max: 5 } },
    default: {},
  } as const;

  it("mapEntries sorts alphabetically and defaults missing/malformed to []", () => {
    const data = { disciplines: { Potence: 2, Celerity: 1, Auspex: 3 } };
    expect(mapEntries(data, field)).toEqual([
      { key: "Auspex", value: 3 },
      { key: "Celerity", value: 1 },
      { key: "Potence", value: 2 },
    ]);
    expect(mapEntries({}, field)).toEqual([]);
    expect(mapEntries({ disciplines: "not an object" }, field)).toEqual([]);
  });

  it("listItems defaults to [] for a missing/malformed field", () => {
    const specs = { id: "s", label: "S", ty: { kind: "list", of: { kind: "text", max_len: 40 } }, default: [] } as const;
    expect(listItems({ s: ["a", "b"] }, specs)).toEqual(["a", "b"]);
    expect(listItems({}, specs)).toEqual([]);
  });

  it("asStringList drops non-string elements", () => {
    expect(asStringList(["a", 1, "b", null])).toEqual(["a", "b"]);
    expect(asStringList("nope")).toEqual([]);
  });
});

describe("visibleFields (staff_only omission)", () => {
  const section: SheetSection = {
    id: "kindred",
    label: "Kindred",
    layout: "list",
    fields: [
      { id: "generation", label: "Generation", ty: { kind: "number", min: 4, max: 15 }, default: 13 },
      {
        id: "true_age",
        label: "True Age",
        ty: { kind: "number", min: 0, max: 2000 },
        default: 0,
        staff_only: true,
      },
    ],
  };

  it("a field the server stripped (staff_only, non-staff viewer) is simply absent from `data` and gets omitted", () => {
    const data = { generation: 13 }; // true_age never arrived — the server stripped it
    const visible = visibleFields(data, section);
    expect(visible.map((f) => f.id)).toEqual(["generation"]);
  });

  it("staff viewer: the field IS present in `data` and gets rendered", () => {
    const data = { generation: 13, true_age: 900 };
    const visible = visibleFields(data, section);
    expect(visible.map((f) => f.id)).toEqual(["generation", "true_age"]);
  });
});

describe("capitalize / approvalKind", () => {
  it("capitalizes the first letter only", () => {
    expect(capitalize("vampire")).toBe("Vampire");
    expect(capitalize("")).toBe("");
  });

  it("approvalKind reads the approval summary", () => {
    expect(approvalKind({ state: "approved", approved: true })).toBe("approved");
    expect(approvalKind({ state: "pending (job #4)", approved: false })).toBe("pending");
    expect(approvalKind({ state: "unapproved", approved: false })).toBe("unapproved");
  });
});

// ---------------------------------------------------------------------------
// A splat section end-to-end: parse a realistic Vampire `wod.sheet` keyframe
// (Disciplines map, Blood Pool number, Generation, a Track health field) and
// walk it exactly as SheetPanel does.
// ---------------------------------------------------------------------------

const vampireSchema = {
  ruleset: "wod",
  version: 2,
  sections: [
    {
      id: "attributes",
      label: "Attributes",
      layout: { grid: { rows: 3, cols: 3 } },
      fields: [{ id: "strength", label: "Strength", ty: { kind: "dots", min: 1, max: 5 }, default: 1 }],
    },
    {
      id: "health",
      label: "Health",
      layout: "tracks",
      fields: [
        {
          id: "health",
          label: "Health",
          ty: {
            kind: "track",
            size: { fixed: 7 },
            levels: [
              { id: "bruised", label: "Bruised", penalty: 0 },
              { id: "hurt", label: "Hurt", penalty: -1 },
            ],
          },
          default: null,
        },
      ],
    },
    {
      id: "disciplines",
      label: "Disciplines",
      layout: "map",
      fields: [
        {
          id: "disciplines",
          label: "Disciplines",
          ty: { kind: "map", value: { kind: "dots", min: 0, max: 5 } },
          default: {},
        },
      ],
    },
    {
      id: "kindred",
      label: "Kindred",
      layout: "list",
      fields: [
        { id: "generation", label: "Generation", ty: { kind: "number", min: 4, max: 15 }, default: 13 },
        { id: "bloodpool", label: "Blood Pool", ty: { kind: "number", min: 0, max: 50 }, default: 10 },
      ],
    },
  ],
};

const vampireKeyframe = {
  target: "#42",
  schema: vampireSchema,
  approved: { state: "approved", approved: true, detail: { approved: { by: 5, at: 12345 } } },
  data: {
    strength: 3,
    health: ["bashing", "lethal", null, null, null, null, null],
    disciplines: { Potence: 2, Celerity: 1 },
    generation: 12,
    bloodpool: 9,
  },
};

describe("a Vampire splat section end-to-end", () => {
  it("parses the schema and the section shapes survive round-trip", () => {
    const schema = parseSheetSchema(vampireSchema) as SheetSchema;
    expect(schema.ruleset).toBe("wod");
    expect(schema.sections.map((s) => s.id)).toEqual(["attributes", "health", "disciplines", "kindred"]);
    expect(schema.sections.find((s) => s.id === "disciplines")).toBeDefined();
  });

  it("parses the full keyframe payload", () => {
    const payload = parseSheetPayload(vampireKeyframe);
    expect(payload).not.toBeNull();
    expect(payload?.target).toBe("#42");
    expect(payload?.approved.approved).toBe(true);
    expect(payload?.data.bloodpool).toBe(9);
  });

  it("renders the Disciplines map as dot rows, Blood Pool as a wide bar, Generation as pips", () => {
    const payload = parseSheetPayload(vampireKeyframe)!;
    const discField = payload.schema.sections.find((s) => s.id === "disciplines")!.fields[0]!;
    expect(mapEntries(payload.data, discField)).toEqual([
      { key: "Celerity", value: 1 },
      { key: "Potence", value: 2 },
    ]);

    const kindred = payload.schema.sections.find((s) => s.id === "kindred")!;
    const generation = kindred.fields.find((f) => f.id === "generation")!;
    const bloodpool = kindred.fields.find((f) => f.id === "bloodpool")!;
    expect(generation.ty.kind).toBe("number");
    expect(generation.ty.kind === "number" && isPipRange(generation.ty.min, generation.ty.max)).toBe(true);
    expect(bloodpool.ty.kind).toBe("number");
    expect(bloodpool.ty.kind === "number" && isPipRange(bloodpool.ty.min, bloodpool.ty.max)).toBe(false);
  });

  it("renders Health as tracked marks with the deepest level surfaced", () => {
    const payload = parseSheetPayload(vampireKeyframe)!;
    const healthField = payload.schema.sections.find((s) => s.id === "health")!.fields[0]!;
    expect(healthField.ty.kind).toBe("track");
    const marks = trackCells(payload.data[healthField.id]);
    expect(marks).toEqual(["bashing", "lethal", null, null, null, null, null]);
    if (healthField.ty.kind === "track") {
      expect(deepestLevel(marks, healthField.ty.levels)?.id).toBe("hurt");
    }
  });
});

describe("parse* survive malformed payloads", () => {
  it("parseSheetPayload rejects a schema-less/garbage payload", () => {
    expect(parseSheetPayload(null)).toBeNull();
    expect(parseSheetPayload("nope")).toBeNull();
    expect(parseSheetPayload({ schema: { ruleset: 5 } })).toBeNull();
  });

  it("parseSheetSchema drops malformed sections/fields instead of throwing", () => {
    const schema = parseSheetSchema({
      ruleset: "wod",
      version: 1,
      sections: [null, 7, { id: "ok", label: "Ok", layout: "list", fields: [null, { id: "f" }] }],
    });
    expect(schema?.sections).toHaveLength(1);
    expect(schema?.sections[0]?.fields).toHaveLength(0); // both malformed fields dropped
  });

  it("parseApproval defaults gracefully", () => {
    expect(parseApproval(null)).toEqual({ state: "unknown", approved: false });
    expect(parseApproval({ state: "approved", approved: true })).toEqual({
      state: "approved",
      approved: true,
      detail: undefined,
    });
  });
});

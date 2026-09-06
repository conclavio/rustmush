// The generic view schema (interface plan I9): defensive parsing, the row
// and cell helpers, the caps, and the D-I-18 row-action rule (a row action
// is always a Prefill line, never a runnable command).

import { describe, expect, it } from "vitest";

import {
  cellText,
  MAX_VIEW_FIELDS,
  MAX_VIEW_ROWS,
  parseGenericView,
  parseViewSchema,
  rowActionLine,
} from "./genericview";

describe("parseViewSchema", () => {
  it("parses a list schema with columns and an action key", () => {
    const s = parseViewSchema({
      id: "jobs",
      version: 1,
      shape: "list",
      columns: [
        { key: "id", label: "#", align: "right" },
        { key: "title" },
      ],
      actionKey: "_open",
    });
    expect(s?.shape).toBe("list");
    expect(s?.columns).toEqual([
      { key: "id", label: "#", align: "right" },
      { key: "title", label: "title", align: "left" },
    ]);
    expect(s?.fields).toEqual([]);
    expect(s?.actionKey).toBe("_open");
  });

  it("parses a detail schema and defaults a field kind to text", () => {
    const s = parseViewSchema({
      id: "job",
      version: 2,
      shape: "detail",
      fields: [{ key: "body", label: "Body", kind: "rich" }, { key: "n", kind: "bogus" }],
    });
    expect(s?.shape).toBe("detail");
    expect(s?.fields).toEqual([
      { key: "body", label: "Body", kind: "rich" },
      { key: "n", label: "n", kind: "text" },
    ]);
    expect(s?.actionKey).toBeNull();
  });

  it("rejects a malformed or unknown-shape schema", () => {
    expect(parseViewSchema({ id: "x", version: 1, shape: "grid" })).toBeNull();
    expect(parseViewSchema({ id: "x", shape: "list" })).toBeNull();
    expect(parseViewSchema("no")).toBeNull();
  });

  it("caps columns and fields", () => {
    const many = Array.from({ length: MAX_VIEW_FIELDS + 10 }, (_, i) => ({ key: `c${i}` }));
    const s = parseViewSchema({ id: "x", version: 1, shape: "list", columns: many });
    expect(s?.columns.length).toBe(MAX_VIEW_FIELDS);
  });
});

describe("parseGenericView", () => {
  it("keeps rows for a list, drops non-objects, and caps the count", () => {
    const many = Array.from({ length: MAX_VIEW_ROWS + 5 }, (_, i) => ({ id: i }));
    const v = parseGenericView({
      schema: { id: "jobs", version: 1, shape: "list", columns: [{ key: "id" }] },
      rows: [{ id: 1 }, "junk", { id: 2 }, ...many],
    });
    expect(v?.rows.length).toBe(MAX_VIEW_ROWS);
    expect(v?.rows[0]).toEqual({ id: 1 });
    expect(v?.rows[1]).toEqual({ id: 2 });
    expect(v?.record).toBeNull();
  });

  it("keeps a record for a detail view", () => {
    const v = parseGenericView({
      schema: { id: "job", version: 1, shape: "detail", fields: [{ key: "body" }] },
      record: { body: "hello" },
    });
    expect(v?.record).toEqual({ body: "hello" });
    expect(v?.rows).toEqual([]);
  });

  it("is null without a usable schema", () => {
    expect(parseGenericView({ rows: [] })).toBeNull();
    expect(parseGenericView(null)).toBeNull();
  });
});

describe("cellText", () => {
  it("renders scalars and empties everything else", () => {
    const row = { s: "x", n: 3, b: true, o: { a: 1 }, nil: null };
    expect(cellText(row, "s")).toBe("x");
    expect(cellText(row, "n")).toBe("3");
    expect(cellText(row, "b")).toBe("true");
    expect(cellText(row, "o")).toBe("");
    expect(cellText(row, "nil")).toBe("");
    expect(cellText(row, "absent")).toBe("");
  });
});

describe("rowActionLine (D-I-18: a row action is a Prefill line)", () => {
  const schema = parseViewSchema({
    id: "jobs",
    version: 1,
    shape: "list",
    columns: [{ key: "title" }],
    actionKey: "_open",
  })!;

  it("returns the prefill line from the action field", () => {
    expect(rowActionLine({ title: "A", _open: "@job 3" }, schema)).toBe("@job 3");
  });

  it("is null when the field is empty, missing, or non-string", () => {
    expect(rowActionLine({ title: "A", _open: "" }, schema)).toBeNull();
    expect(rowActionLine({ title: "A" }, schema)).toBeNull();
    expect(rowActionLine({ title: "A", _open: 3 }, schema)).toBeNull();
  });

  it("is null when the schema declares no action", () => {
    const noAction = parseViewSchema({ id: "x", version: 1, shape: "list", columns: [{ key: "a" }] })!;
    expect(rowActionLine({ a: "1", _open: "x" }, noAction)).toBeNull();
  });
});

// Render-equivalence and projection (interface plan I9, verify C and P): the
// generic list carries the same information a legal-actions feed does — the
// label shown and the Prefill line each row runs — so an actions-shaped feed
// needs no bespoke renderer; and a row field the schema does not name (an
// opaque offer token) is structurally unreachable, so it can never leak.
describe("generic view reproduces an actions list without leaking offers", () => {
  const actions = [
    { offer: "secret-token", presentation: { verb: "brace", label: "Brace", help: "Hold." } },
    { offer: "another-secret", presentation: { verb: "aim", label: "Aim", help: "Steady." } },
  ];
  // The adapter a future actions-shaped `.view` producer would use: label as
  // a column, the `+act <verb>` line as the action; the offer token is never
  // placed on a row.
  const view = parseGenericView({
    schema: {
      id: "actions",
      version: 1,
      shape: "list",
      columns: [{ key: "label", label: "Action" }],
      actionKey: "act",
    },
    rows: actions.map((a) => ({ label: a.presentation.label, act: `+act ${a.presentation.verb}` })),
  })!;

  it("shows each label and runs the +act line", () => {
    expect(view.rows.map((r) => cellText(r, "label"))).toEqual(["Brace", "Aim"]);
    expect(view.rows.map((r) => rowActionLine(r, view.schema))).toEqual([
      "+act brace",
      "+act aim",
    ]);
  });

  it("only ever reads declared columns, so an offer token cannot render", () => {
    const rendered = view.rows
      .flatMap((r) => view.schema.columns.map((c) => cellText(r, c.key)))
      .join(" ");
    expect(rendered).not.toContain("secret");
    expect(rendered).not.toContain("offer");
  });
});

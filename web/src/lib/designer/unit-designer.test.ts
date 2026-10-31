import { describe, expect, it } from "vitest";

import {
  DRAFT_KEY,
  type Draft,
  History,
  draftFrom,
  type ExportFile,
  exportFiles,
  figures,
  loadDraft,
  models,
  rename,
  saveDraft,
  sections,
  setFacetArmor,
  setInternals,
  setMaxSpeed,
  setModel,
  setWeapon,
  setWeaponName,
  slugOf,
  templates,
  testHit,
  weaponComponents,
  weapons,
} from "./unit-designer";
import { UnitDesignRpc } from "./rpc";

function tank(): Draft {
  const t = templates().find((x) => x.id === "tacsim.unit.line_tank");
  if (!t) throw new Error("the line tank ships");
  return draftFrom(t, "Line Tank Mk2");
}

describe("the unit designer's engine (doc 31 cut UD.2)", () => {
  it("reads the shipped catalogue", () => {
    expect(templates().length).toBeGreaterThanOrEqual(14);
    expect(weapons().map((w) => w.id)).toContain("tacsim.weapon.laser_mk1");
    expect(models().map((m) => m.id)).toContain("mc.st.tank_lvl2");
    const t = templates().find((x) => x.id === "tacsim.unit.line_tank");
    expect(t?.look?.model).toBe("mc.st.tank_lvl2");
  });

  it("starts a draft from a template under a new id, its look pointed at it", () => {
    const d = tank();
    expect(d.id).toBe("tacsim.unit.design.line_tank_mk2");
    expect(d.look.units).toEqual([d.id]);
    expect(d.rules.label).toBe("Line Tank Mk2");
    expect(slugOf("  Heavy / Walker!! ")).toBe("heavy_walker");
    expect(rename(d, "Other").look.units).toEqual(["tacsim.unit.design.other"]);
  });

  it("edits armour, internals, weapons, speed and names without touching the template", () => {
    const t = templates().find((x) => x.id === "tacsim.unit.line_tank")!;
    const before = JSON.stringify(t.def);
    let d = tank();
    d = setFacetArmor(d, "hull", "front", 450);
    d = setInternals(d, "turret", 77);
    d = setWeapon(d, "cannon", "tacsim.weapon.laser_mk1");
    d = setWeaponName(d, "cannon", "LZ");
    d = setMaxSpeed(d, 1200);
    const hull = sections(d).find((s) => s.id === "hull")!;
    expect(hull.facets.find((f) => f.id === "front")?.armor).toBe(450);
    expect(sections(d).find((s) => s.id === "turret")?.internals).toBe(77);
    expect(weaponComponents(d).find((w) => w.id === "cannon")?.weapon).toBe("tacsim.weapon.laser_mk1");
    expect((d.look.weapons as Record<string, string>).cannon).toBe("LZ");
    expect(figures(d).speedCms).toBe(1200);
    expect(JSON.stringify(t.def)).toBe(before);
  });

  it("puts a look on a model at the model's height, its width within the check's bounds", () => {
    const biped = models().find((m) => m.id === "mc.lt.fox_biped")!;
    const d = setModel(tank(), biped);
    expect(d.look.model).toBe(biped.id);
    const [w, h] = d.look.size_cm as number[];
    expect(h).toBe(biped.size_cm[1]);
    expect(w).toBeGreaterThanOrEqual(Math.ceil(biped.size_cm[0] / 2));
    expect(w).toBeLessThanOrEqual(biped.size_cm[0] * 2);
  });

  it("says what a destroyed section costs (the test hit)", () => {
    const d = tank();
    const turret = testHit(d, "turret");
    expect(turret.some((h) => h.includes("every weapon on turret"))).toBe(true);
    expect(testHit(d, "nonesuch")).toEqual([]);
  });

  it("undoes and redoes, the redo branch going on a new edit", () => {
    const h = new History(3);
    const a = tank();
    const b = setInternals(a, "hull", 1);
    h.push(a);
    expect(h.canUndo).toBe(true);
    const back = h.undo(b)!;
    expect(sections(back).find((s) => s.id === "hull")?.internals).toBe(sections(a).find((s) => s.id === "hull")?.internals);
    expect(h.redo(back)).not.toBeNull();
    h.push(b);
    expect(h.canRedo).toBe(false);
    for (let i = 0; i < 5; i++) h.push(a);
    let n = 0;
    while (h.undo(a)) n++;
    expect(n).toBe(3);
  });

  it("keeps the working draft, and survives storage that refuses", () => {
    const store = new Map<string, string>();
    const storage = { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) };
    const d = tank();
    saveDraft(storage, d);
    expect(store.has(DRAFT_KEY)).toBe(true);
    expect(loadDraft(storage)?.id).toBe(d.id);
    store.set(DRAFT_KEY, "{not json");
    expect(loadDraft(storage)).toBeNull();
    saveDraft({ setItem: () => { throw new Error("full"); } }, d);
    expect(loadDraft(null)).toBeNull();
  });

  it("exports the server's pack and the look as the tree formats them", () => {
    const d = tank();
    const pack = { manifest: { id: "tacsim.pack.line_tank_mk2" }, entries: {} };
    const [rules, look] = exportFiles(d, pack) as [ExportFile, ExportFile];
    expect(rules.path).toBe("crates/tacsim/content/units/line_tank_mk2.json");
    expect(look.path).toBe("crates/mush-pixel/looks/line_tank_mk2.json");
    expect(rules.text.endsWith("}\n")).toBe(true);
    expect(JSON.parse(look.text).units).toEqual([d.id]);
    expect(look.text).toContain('\n  "units": [');
  });
});

describe("the unit_design invoke client", () => {
  type Handler = (e: { d: { id: number; result: { service: string; result: unknown } } }) => void;
  function fakeClient(answer: (id: number) => unknown, online = true) {
    let handler: Handler | null = null;
    const sent: unknown[] = [];
    let next = 1;
    const client = {
      on: (_t: string, fn: Handler) => {
        handler = fn;
        return () => {
          handler = null;
        };
      },
      invoke: (request: unknown) => {
        if (!online) return null;
        sent.push(request);
        const id = next++;
        queueMicrotask(() => handler?.({ d: { id, result: { service: "unit_design", result: answer(id) } } }));
        return id;
      },
    };
    return { client, sent };
  }

  it("sends a check and resolves with the server's answer", async () => {
    const { client, sent } = fakeClient(() => ({ status: "checked", refusals: ["no"], pack: null }));
    const rpc = new UnitDesignRpc(client as never);
    const answer = await rpc.request({ op: "check", id: "tacsim.unit.x", version: 1, rules: {} });
    expect(answer).toEqual({ status: "checked", refusals: ["no"], pack: null });
    expect(sent).toEqual([{ service: "unit_design", request: { op: "check", id: "tacsim.unit.x", version: 1, rules: {} } }]);
    rpc.close();
  });

  it("answers offline as an error rather than throwing", async () => {
    const { client } = fakeClient(() => null, false);
    const rpc = new UnitDesignRpc(client as never);
    const answer = await rpc.request({ op: "check", id: "tacsim.unit.x", version: 1, rules: {} });
    expect(answer.status).toBe("error");
  });
});
